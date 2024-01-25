use bastion::{Bastion, Callbacks, run};
use bastion::children::Children;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use futures::lock::Mutex;
use log::{error, info};
use bastion::distributor::Distributor;
use bastion::prelude::SupervisionStrategy;
use std::ops::{Deref, DerefMut};
use crate::steady::channel::ChannelBuilder;
use crate::steady::{config, telemetry};
use crate::steady::monitor::SteadyMonitor;
use crate::steady::telemetry::metrics_collector::CollectorDetail;

pub struct SteadyGraph {
    channel_count: Arc<AtomicUsize>,
    monitor_count: usize,
    //used by collector but could grow if we get new actors at runtime
    all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,

    runtime_state: Arc<Mutex<GraphRuntimeState>>,

}

//for testing only
#[cfg(test)]
impl SteadyGraph  {



    pub fn new_test_monitor(self: &mut Self, name: & 'static str ) -> SteadyMonitor
    {
        let id = self.monitor_count;
        self.monitor_count += 1;

        let channel_count = self.channel_count.clone();
        let all_telemetry_rx = self.all_telemetry_rx.clone();
        SteadyMonitor {
            channel_count,
            name,
            ctx: None,
            id,
            all_telemetry_rx,
            runtime_state: self.runtime_state.clone()
        }
    }
}


impl SteadyGraph {

    pub(crate) fn start(&mut self) {
        Bastion::start(); //start the graph
        let mut guard = run!(self.runtime_state.lock());
        let state = guard.deref_mut();
        *state = GraphRuntimeState::Running;
    }

    pub(crate) fn request_shutdown(self: &mut Self) {

        let mut guard = run!(self.runtime_state.lock());
        let state = guard.deref_mut();
        *state = GraphRuntimeState::StopRequested;

    }

    /// add new children actors to the graph
    pub fn add_to_graph<F,I>(&mut self, name: & 'static str, c: Children, init: I ) -> Children
        where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
              F: Future<Output = Result<(),()>> + Send + 'static ,  {

        SteadyGraph::configure_for_graph(self, name, c, init)
    }

    /// create a new graph for the application typically done in main
    pub(crate) fn new() -> SteadyGraph {
        SteadyGraph {
            channel_count: Arc::new(AtomicUsize::new(0)),
            monitor_count: 0, //this is the count of all monitors
            all_telemetry_rx: Arc::new(Mutex::new(Vec::new())), //this is all telemetry receivers
            runtime_state: Arc::new(Mutex::new(GraphRuntimeState::Building))
        }
    }

    /// create new single channel before we build the graph actors

    pub fn channel_builder<T>(&mut self, capacity: usize ) -> ChannelBuilder<T> {
        ChannelBuilder::new(self.channel_count.clone(), capacity)
    }


    fn configure_for_graph<F,I>(graph: & mut SteadyGraph, name: & 'static str, c: Children, init: I ) -> Children
            where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
                  F: Future<Output = Result<(),()>> + Send + 'static ,
        {
            let result = {
                let init_clone = init.clone();

                let id = graph.monitor_count;
                graph.monitor_count += 1;
                let telemetry_tx = graph.all_telemetry_rx.clone();
                let channel_count = graph.channel_count.clone();
                let runtime_state = graph.runtime_state.clone();

                let callbacks = Callbacks::new().with_after_stop(|| {
                   //TODO: new telemetry counting actor restarts
                });
                c.with_exec(move |ctx| {
                        let init_fn_clone = init_clone.clone();
                        let channel_count = channel_count.clone();
                        let telemetry_tx = telemetry_tx.clone();
                        let runtime_state = runtime_state.clone();

                        async move {
                            //this telemetry now owns this monitor

                            let monitor = SteadyMonitor {
                                runtime_state:  runtime_state.clone(),
                                channel_count: channel_count.clone(),
                                name,
                                ctx: Some(ctx),
                                id,
                                all_telemetry_rx: telemetry_tx.clone(),
                            };


                            match init_fn_clone(monitor).await {
                                Ok(_) => {
                                    info!("Actor {:?} finished ", name);
                                },
                                Err(e) => {
                                    error!("{:?}", e);
                                    return Err(e);
                                }
                            }
                            Ok(())
                        }
                    })
                    .with_callbacks(callbacks)
                    .with_name(name)
            };

            #[cfg(test)] {
                result.with_distributor(Distributor::named(format!("testing-{name}")))
            }
            #[cfg(not(test))] {
                result
            }
        }

    /*

    pub(crate) fn callbacks() -> Callbacks {

        //is the call back recording events for the elemetry?
        Callbacks::new()       .with_before_start( || {
                //TODO: record the name? of the telemetry on the graph needed?
                // info!("before start");
            })
            .with_after_start( || {
                //TODO: change telemetry to started if needed
                // info!("after start");
            })
            .with_after_stop( || {
                //TODO: record this telemetry has stopped on the graph
                  info!("after stop");
            })
            .with_after_restart( || {
                //TODO: record restart count on the graph
    //info!("after restart");
            })
    }*/
    pub(crate) fn init_telemetry(&mut self) {


        //The Troupe is restarted together if one telemetry fails
        let _ = Bastion::supervisor(|supervisor| {
            let supervisor = supervisor.with_strategy(SupervisionStrategy::OneForAll);

            let mut outgoing = None;

            let supervisor = if config::TELEMETRY_SERVER {
                //build channel for DiagramData type
                let (tx,rx) = self.channel_builder(config::REAL_CHANNEL_LENGTH_TO_FEATURE)
                                  .with_labels(&["steady-telemetry"],true)
                                  .build();

                outgoing = Some(tx);
                supervisor.children(|children| {
                    self.add_to_graph("telemetry-polling"
                                      , children.with_redundancy(0)
                                      , move |monitor|
                                              telemetry::metrics_server::run(monitor
                                                                             , rx.clone()
                                              )
                    )
                })
            } else {
                supervisor
            };


            let senders_count:usize = {
                let guard = run!(self.all_telemetry_rx.lock());
                let v = guard.deref();
                v.len()
            };

            //only spin up the metrics collector if we have a consumer
            //OR if some actors are sending data we need to consume
            if config::TELEMETRY_SERVER || senders_count>0 {

                supervisor.children(|children| {
                    //we create this child last so we can clone the rx_vec
                    //and capture all the telemetry actors as well
                    let all_tel_rx = self.all_telemetry_rx.clone(); //using Arc here

                    SteadyGraph::configure_for_graph(self, "telemetry-collector"
                                                     , children.with_redundancy(0)
                                                     , move |monitor| {

                            let all_rx = all_tel_rx.clone();
                            telemetry::metrics_collector::run(monitor
                                                              , all_rx
                                                              , outgoing.clone()
                            )
                        }
                    )
                }
                )
            } else {
                supervisor
            }

        }).expect("Telemetry supervisor creation error.");
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum GraphRuntimeState {
    Building,
    Running,
    StopRequested, //not yet confirmed by all nodes
    StopInProgress, //confirmed by all nodes and now stopping
    Stopped,
}
