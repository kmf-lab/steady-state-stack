
use std::any::type_name;
use std::future::Future;
use std::ops::{Deref, DerefMut, Sub};
use std::sync::{Arc};
use std::time::{Duration, Instant};
use bastion::{Callbacks, run};
use bastion::children::Children;
use bastion::prelude::*;




use futures_timer::Delay;
use log::{error, info};
use petgraph::matrix_graph::Zero;
use crate::steady::telemetry::metrics_collector::DiagramData;
use crate::steady_feature::steady_feature;

use async_ringbuf::{AsyncRb, traits::*};
use async_ringbuf::wrap::{AsyncCons, AsyncProd};
use async_std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use ringbuf::storage::Heap;

const MAX_TELEMETRY_ERROR_RATE_SECONDS: usize = 60;

type ChannelBacking<T> = Heap<T>;

type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;
type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;

    //TODO: we want to use Static but will use heap as first step
    //TODO: we must use static for all telemetry work.
    //let mut rb = AsyncRb::<Static<T, 12>>::default().split_ref()
    //   AsyncRb::<ChannelBacking<T>>::new(cap).split_ref()



////

mod telemetry {
    pub mod metrics_collector;
    pub mod telemetry_logging;
    pub mod telemetry_streaming;
    pub mod telemetry_polling;

}

pub struct SteadyGraph {
    channel_count: Arc<AtomicUsize>,
    monitor_count: usize,

    //used by collector but could grow if we get new actors at runtime
    all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,

}
struct CollectorDetail {
    telemetry_take: Box<dyn RxTel>,
    name: &'static str,
    monitor_id: usize
}

pub struct SteadyMonitor {
    id: usize, //unique identifier for this child group
    name: & 'static str,

    ctx: Option<BastionContext>,

    channel_count: Arc<AtomicUsize>,
    all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>
}

pub struct LocalMonitor<const RX_LEN: usize, const TX_LEN: usize> {
    id: usize, //unique identifier for this child group
    name: & 'static str,
    ctx: Option<BastionContext>,
    telemetry_send_tx: Option<SteadyTelemetrySend<TX_LEN>>,
    telemetry_send_rx: Option<SteadyTelemetrySend<RX_LEN>>,
    last_instant:      Instant,
//TODO: try removing these RwLocks again this should be local only.
}

pub struct SteadyTelemetryRx<const RXL: usize, const TXL: usize> {
    send: Option<SteadyTelemetryTake<TXL>>,
    take: Option<SteadyTelemetryTake<RXL>>,
}

struct SteadyTelemetryTake<const LENGTH: usize> {
    rx: Arc<Mutex<SteadyRx<[usize; LENGTH]>>>,
    map: [usize; LENGTH],
}
//NOTE: we could get a read access to the SteadyRx and SteadyTx
//      BUT this could cause stalls in the application so instead
//      this data is gathered on the developers schedule when batch
//      is called
//

struct SteadyTelemetrySend<const LENGTH: usize> {
    tx: Arc<Mutex<SteadyTx<[usize; LENGTH]>>>,
    count: [usize; LENGTH],
    limits: [usize; LENGTH],
    last_telemetry_error: Instant,
}

impl <const LENGTH: usize> SteadyTelemetrySend<LENGTH> {
    pub fn new(tx: Arc<Mutex<SteadyTx<[usize; LENGTH]>>>,
               count: [usize; LENGTH],
               limits: [usize; LENGTH]) -> SteadyTelemetrySend<LENGTH> {
        SteadyTelemetrySend{
             tx,count,limits
            ,last_telemetry_error: Instant::now().sub(Duration::from_secs(1+MAX_TELEMETRY_ERROR_RATE_SECONDS as u64))
        }
    }
}

pub struct SteadyRx<T> {
    id: usize,
    label: &'static [& 'static str],
    batch_limit: usize,
    rx: InternalReceiver<T>,
    local_idx: usize,
}



pub struct SteadyTx<T> {
    id: usize,
    batch_limit: usize,
    tx: InternalSender<T>,
    label: &'static [& 'static str],
    local_idx: usize,
    actor_name: Option<&'static str>,
}

////////////////////////////////////////
//////////////////////////////////////////////
////////////////////////////////////////

macro_rules! guard {
    ($lock:expr) => {
        $lock.lock().await
    };
}

macro_rules! ref_mut {
    ($guard:expr) => {
        std::ops::DerefMut::deref_mut(&mut $guard)
    };
}
///////////////////
impl <const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {

    pub(crate) fn ctx(&self) -> Option<&BastionContext> {
        if let Some(ctx) = &self.ctx {
            Some(ctx)
        } else {
            None
        }
    }

    pub async fn relay_stats_all(&mut self) {

        // do not run any faster than the framerate of the telemetry can consume
        //TODO: this is not right needs alignment with the periodic counter and other calls
        //let run_duration: Duration = Instant::now().duration_since(self.monitor.last_instant);
        //if run_duration.as_millis() as usize >= steady_feature::MIN_TELEMETRY_CAPTURE_RATE_MS
        {
            let is_in_bastion:bool = {self.ctx().is_some()};

            if let Some(ref mut send_tx) = self.telemetry_send_tx {
                // switch to a new vector and send the old one
                // we always clear the count so we can confirm this in testing

                if send_tx.count.iter().any(|x| !x.is_zero()) {
                    //we only send the result if we have a context, ie a graph we are monitoring
                    if is_in_bastion {

                        let mut lock_guard = guard!(send_tx.tx);
                        let tx = ref_mut!(lock_guard);
                        match tx.try_send(send_tx.count) {
                            Ok(_)  => {
                                send_tx.count.fill(0);
                                if usize::MAX != tx.local_idx { //only record those turned on (init)
                                    send_tx.count[tx.local_idx] = 1;
                                }
                            },
                            Err(a) => {
                                let now = Instant::now();
                                let dif = now.duration_since(send_tx.last_telemetry_error);
                                if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                    error!("full telemetry channel detected upon tx from actor: {} value:{:?} "
                                       , self.name, a);
                                    //store time to do this again later.
                                    send_tx.last_telemetry_error = now;
                                }
                            }
                        }

                    } else {
                        send_tx.count.fill(0);
                    }
                }
            }

            if let Some(ref mut send_rx) = self.telemetry_send_rx {
                if send_rx.count.iter().any(|x| !x.is_zero()) {
                        //we only send the result if we have a context, ie a graph we are monitoring
                        if is_in_bastion {

                            let mut lock_guard = guard!(send_rx.tx);
                            let rx = ref_mut!(lock_guard);
                            match rx.try_send(send_rx.count) {
                                Ok(_)  => {
                                    send_rx.count.fill(0);
                                    if usize::MAX != rx.local_idx { //only record those turned on (init)
                                        send_rx.count[rx.local_idx] = 1;
                                    }
                                },
                                Err(a) => {
                                    let now = Instant::now();
                                    let dif = now.duration_since(send_rx.last_telemetry_error);
                                    if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                         error!("full telemetry channel detected upon rx from actor: {} value:{:?} "
                                             , self.name, a);
                                        //store time to do this again later.
                                        send_rx.last_telemetry_error = now;
                                    }
                                }
                            }


                        } else {
                            send_rx.count.fill(0);
                        }
                }
            }
        }
    }


    pub async fn relay_stats_periodic(self: &mut Self, duration_rate: Duration) {

        assert_eq!(true, duration_rate.ge(&Duration::from_millis(steady_feature::MIN_TELEMETRY_CAPTURE_RATE_MS as u64)));

        let run_duration: Duration = {
                Instant::now().duration_since(self.last_instant).clone()
            };

        run!(
            async {
        Delay::new(duration_rate.saturating_sub(run_duration)).await;

            }
        );

        {
            self.last_instant = Instant::now();
            self.relay_stats_all().await;
        }


    }


    pub async fn relay_stats_batch(self: &mut Self) {
        //only relay if one of the channels has reached or passed the capacity
        let doit =
                if let Some(send) = &self.telemetry_send_tx {
                    if send.count.iter().zip(send.limits.iter()).any(|(c,l)|c>=l) {
                        true
                    } else {
                        if let Some(send) = &self.telemetry_send_rx {
                            send.count.iter().zip(send.limits.iter()).any(|(c,l)| c>=l)
                        } else {
                            false
                        }
                    }
                } else {
                    if let Some(send) = &self.telemetry_send_rx {
                        send.count.iter().zip(send.limits.iter()).any(|(c,l)| c>=l)
                    } else {
                        false
                    }
                };
        if doit {
            self.relay_stats_all().await;
        }
    }

    pub fn relay_stats_tx_set_custom_batch_limit<T>(self: &mut Self, tx: &SteadyTx<T>, threshold: usize) {
        if let Some(ref mut send) = self.telemetry_send_tx {
            if usize::MAX != tx.local_idx {
                send.limits[tx.local_idx] = threshold;
            }
        }
    }

    pub fn relay_stats_rx_set_custom_batch_limit<T>(self: &mut Self, rx: &SteadyRx<T>, threshold: usize) {
        if let Some(ref mut send) = self.telemetry_send_rx {
            if usize::MAX != rx.local_idx {
                send.limits[rx.local_idx] = threshold;
            }
        }
    }

    pub fn try_take<T>(& mut self, this: & mut SteadyRx<T>) -> Option<T> {
        match this.try_take() {
            Some(msg) => {
                if let Some(ref mut telemetry) = self.telemetry_send_rx {
                    if usize::MAX != this.local_idx { //only record those turned on (init)
                        let count = telemetry.count[this.local_idx];
                        telemetry.count[this.local_idx] = count.saturating_add(1);
                    }
                }
                Some(msg)
            },
            None => {None}
        }
    }
    pub fn try_peek<'a,T>(&'a mut self, this: &'a mut SteadyRx<T>) -> Option<&T> {
        this.try_peek()  //nothing to record since nothing moved. TODO: revisit this
    }
    pub fn is_empty<T>(& mut self, this: & mut SteadyRx<T>) -> bool {
        this.is_empty()
    }

    pub async fn take_async<T>(& mut self, this: & mut SteadyRx<T>) -> Result<T, String> {
        match this.take_async().await {
            Ok(result) => {
                if let Some(ref mut telemetry) = self.telemetry_send_rx {
                        if usize::MAX != this.local_idx { //only record those turned on (init)
                            let count = telemetry.count[this.local_idx];
                            telemetry.count[this.local_idx] = count.saturating_add(1);
                        }
                }
                Ok(result)
            },
            Err(error_msg) => {
                error!("Unexpected error recv_async: {} {}", error_msg, self.name);
                Err(error_msg)
            }
        }
    }

    pub fn try_send<T>(self: & mut Self, this: & mut SteadyTx<T>, msg: T) -> Result<(), T> {
        match this.try_send(msg) {
            Ok(_) => {
                if let Some(ref mut telemetry) = self.telemetry_send_tx {
                    if usize::MAX != this.local_idx { //only record those turned on (init)
                        let count = telemetry.count[this.local_idx];
                        telemetry.count[this.local_idx] = count.saturating_add(1);
                    }
                }
                Ok(())
            },
            Err(sensitive) => {
                error!("Unexpected error try_send  actor: {} type: {}"
                    , self.name, type_name::<T>());
                Err(sensitive)
            }
        }
    }
    pub fn is_full<T>(self: & mut Self,this: & mut SteadyTx<T>) -> bool {
        this.is_full()
    }

    pub async fn send_async<T>(self: & mut Self, this: & mut SteadyTx<T>, a: T) -> Result<(), T> {
       match this.send_async(a).await {
           Ok(_) => {
               if let Some(ref mut telemetry) = self.telemetry_send_tx {
                   if usize::MAX != this.local_idx { //only record those turned on (init)
                       let count = telemetry.count[this.local_idx];
                       telemetry.count[this.local_idx] = count.saturating_add(1);
                   }
               }
               return Ok(())
           },
           Err(sensitive) => {
               error!("Unexpected error send_async  actor: {} type: {}"
                   , self.name, type_name::<T>());
               Err(sensitive)
           }
       }

    }


}


//TODO: lock and unwrap the actual SteadyTx and Rx values.. for use


//TODO: parallel children are a problem for the new single send recieve channels.

impl SteadyMonitor {

    pub fn init_stats<const RX_LEN: usize, const TX_LEN: usize>(self
                                   , rx_tag: &mut [& mut dyn RxDef; RX_LEN]
                                   , tx_tag: &mut [& mut dyn TxDef; TX_LEN]
    ) -> LocalMonitor<RX_LEN,TX_LEN> {

        let mut rx_batch_limit = [0; RX_LEN];
        let mut map_rx = [0; RX_LEN];
        rx_tag.iter_mut()
            .enumerate()
            .for_each(|(c, rx)| {
                rx_batch_limit[c] = rx.batch_limit();
                map_rx[c] = rx.id();
                rx.set_local_id(c);
            });


        let mut tx_batch_limit = [0; TX_LEN];
        let mut map_tx = [0; TX_LEN];
        tx_tag.iter_mut()
            .enumerate()
            .for_each(|(c, tx)| {
                tx.set_local_id(c, self.name);
                tx_batch_limit[c] = tx.batch_limit();
                map_tx[c] = tx.id();
            });


        //NOTE: if this child actor is monitored so we will create the appropriate channels

        let rx_tuple: (Option<SteadyTelemetrySend<RX_LEN>>, Option<SteadyTelemetryTake<RX_LEN>>)
            = if RX_LEN.is_zero() {
                (None,None)
            } else {
                let (telemetry_send_rx, mut telemetry_take_rx)
                    = build_channel(self.channel_count.fetch_add(1,Ordering::SeqCst)
                                    ,steady_feature::CHANNEL_LENGTH_TO_COLLECTOR
                                    , &["steady-telemetry"]
                                    );
                ( Some(SteadyTelemetrySend{tx: telemetry_send_rx, count: [0; RX_LEN], limits: rx_batch_limit
                    , last_telemetry_error: Instant::now().sub(Duration::from_secs(1+MAX_TELEMETRY_ERROR_RATE_SECONDS as u64)),
                     })
                 ,Some(SteadyTelemetryTake{rx: telemetry_take_rx, map: map_rx })  )
            };

        let tx_tuple: (Option<SteadyTelemetrySend<TX_LEN>>, Option<SteadyTelemetryTake<TX_LEN>>)
            = if TX_LEN.is_zero() {
                 (None,None)
        } else {
            let (telemetry_send_tx, telemetry_take_tx)
                = build_channel(self.channel_count.fetch_add(1,Ordering::SeqCst)
                                , steady_feature::CHANNEL_LENGTH_TO_COLLECTOR
                                , &["steady-telemetry"]
                                );
            ( Some(SteadyTelemetrySend{tx: telemetry_send_tx, count: [0; TX_LEN], limits: tx_batch_limit
                                , last_telemetry_error: Instant::now().sub(Duration::from_secs(1+MAX_TELEMETRY_ERROR_RATE_SECONDS as u64))},)
              ,Some(SteadyTelemetryTake{rx: telemetry_take_tx, map: map_tx })  )
        };


        let details = CollectorDetail{
              name: self.name
            , monitor_id: self.id
            , telemetry_take: Box::new(SteadyTelemetryRx {
                                            send: tx_tuple.1,
                                            take: rx_tuple.1,
            })
        };

        //need to hand off to the collector
        run!(async{
                let mut shared_vec_guard = self.all_telemetry_rx.lock().await;
                let shared_vec = shared_vec_guard.deref_mut();
                let index:Option<usize> = shared_vec.iter()
                                        .enumerate()
                                        .find(|(_,x)|x.monitor_id == self.id && x.name == self.name)
                                        .map(|(idx,_)|idx);
                if let Some(idx) = index {
                    shared_vec[idx] = details; //replace old telemetry channels
                } else {
                    shared_vec.push(details); //add new telemetry channels
                }
            });

        let telemetry_send_rx = rx_tuple.0;
        let telemetry_send_tx = tx_tuple.0;

         // this is my locked version for this specific thread
        LocalMonitor::<RX_LEN, TX_LEN> {
            telemetry_send_rx,  telemetry_send_tx,
            last_instant: Instant::now(),
            id: self.id,
            name: self.name,
            ctx: self.ctx,
        }
    }

    //TODO: add methods later to instrument non await blocks for cpu usage chart

}

//TODO: refactor into struct later
pub(crate) fn callbacks() -> Callbacks {

    //is the call back recording events for the elemetry?
    Callbacks::new()
        .with_before_start( || {
            //TODO: record the name? of the actor on the graph needed?
            // info!("before start");
        })
        .with_after_start( || {
            //TODO: change actor to started if needed
            // info!("after start");
        })
        .with_after_stop( || {
            //TODO: record this actor has stopped on the graph
            //  info!("after stop");
        })
        .with_after_restart( || {
            //TODO: record restart count on the graph
//info!("after restart");
        })
}





fn build_new_monitor(name: & 'static str, ctx: Option<BastionContext>
                           , id: usize
                           , channel_count: Arc<AtomicUsize>
                           , all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>
) -> SteadyMonitor {

    SteadyMonitor {
        channel_count,
        name,
        ctx,
        id,
        all_telemetry_rx,
    }
}

//for testing only
#[cfg(test)]
impl SteadyGraph  {
    pub fn new_test_monitor(self: &mut Self, name: & 'static str ) -> SteadyMonitor
    {
        let id = self.monitor_count;
        self.monitor_count += 1;

        build_new_monitor(name, None, id, self.channel_count.clone()
                          , self.all_telemetry_rx.clone())

    }
}



fn build_channel<T>(
                     id: usize, cap: usize, labels: & 'static [& 'static str]
                    ) -> (Arc<Mutex<SteadyTx<T>>>, Arc<Mutex<SteadyRx<T>>>) {
    let rb = AsyncRb::<ChannelBacking<T>>::new(cap);
    let (tx, rx): (AsyncProd<Arc<AsyncRb<Heap<T>>>>, AsyncCons<Arc<AsyncRb<Heap<T>>>>) = rb.split();
    (  Arc::new(Mutex::new(SteadyTx::new(id, tx, cap, labels, None)))
     , Arc::new(Mutex::new(SteadyRx::new(id, rx, cap, labels))) )
}

impl SteadyGraph {

    /// add new children actors to the graph
    pub fn add_to_graph<F,I>(self: &mut Self, name: & 'static str, c: Children, init: I ) -> Children
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
        }
    }

    /// create new single channel before we build the graph actors
    pub fn new_channel<T>(&mut self
                          , cap: usize
                          , labels: &'static [& 'static str])
                                 -> (Arc<Mutex<SteadyTx<T>>>, Arc<Mutex<SteadyRx<T>>>) {
        build_channel( self.channel_count.fetch_add(1,Ordering::SeqCst)
                       , cap, labels)
    }



    fn configure_for_graph<F,I>(graph: & mut SteadyGraph, name: & 'static str, c: Children, init: I ) -> Children
            where I: Fn(SteadyMonitor) -> F + Send + 'static + Clone,
                  F: Future<Output = Result<(),()>> + Send + 'static ,
        {
            let result = {
                let init_clone = init.clone();

                let id = graph.monitor_count;
                graph.monitor_count += 1;
                let (id, telemetry_tx) = (id, graph.all_telemetry_rx.clone());
                let channel_count = graph.channel_count.clone();

                c.with_callbacks(callbacks())
                    .with_exec(move |ctx| {
                        let init_fn_clone = init_clone.clone();
                        let channel_count = channel_count.clone();
                        let telemetry_tx = telemetry_tx.clone();
                        async move {
                            //this actor now owns this monitor
                            let monitor = build_new_monitor(name
                                         , Some(ctx)
                                         , id
                                         , channel_count.clone()
                                         , telemetry_tx.clone());

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
                    .with_name(name)
            };

            #[cfg(test)] {
                result.with_distributor(Distributor::named(format!("testing-{name}")))
            }
            #[cfg(not(test))] {
                result
            }
        }


    pub(crate) fn init_telemetry(&mut self) {


        //The Troupe is restarted together if one actor fails
        let _ = Bastion::supervisor(|supervisor| {
            let supervisor = supervisor.with_strategy(SupervisionStrategy::OneForAll);

            let mut outgoing = Vec::new();

            let supervisor = if steady_feature::TELEMETRY_LOGGING {
                let (tx, rx) = self.new_channel::<DiagramData>(steady_feature::CHANNEL_LENGTH_TO_FEATURE
                                                               , &["steady-telemetry"]);
                outgoing.push(tx);
                supervisor.children(|children| {
                    self.add_to_graph("telemetry-logging"
                                      , children.with_redundancy(0)
                                      , move |monitor|
                                              telemetry::telemetry_logging::run(monitor
                                                                            , rx.clone()
                                          )

                    )
                })
            } else {
                supervisor
            };

            let supervisor = if steady_feature::TELEMETRY_STREAMING {
                let (tx, rx) = self.new_channel::<DiagramData>(steady_feature::CHANNEL_LENGTH_TO_FEATURE
                                                               , &["steady-telemetry"]);
                outgoing.push(tx);
                supervisor.children(|children| {
                    self.add_to_graph("telemetry-streaming"
                                      , children.with_redundancy(0)
                                      , move |monitor|
                                             telemetry::telemetry_streaming::run(monitor
                                                                                 , rx.clone()
                                             )

                    )
                })
            } else {
                supervisor
            };

            let supervisor = if steady_feature::TELEMETRY_POLLING {
                let (tx, rx) = self.new_channel::<DiagramData>(steady_feature::CHANNEL_LENGTH_TO_FEATURE
                                                               , &["steady-telemetry"]);
                outgoing.push(tx);
                supervisor.children(|children| {
                    self.add_to_graph("telemetry-polling"
                                      , children.with_redundancy(0)
                                      , move |monitor|
                                              telemetry::telemetry_polling::run(monitor
                                                                                , rx.clone()
                                              )

                    )
                })
            } else {
                supervisor
            };

            let we_have_senders = {
                let guard = run!(self.all_telemetry_rx.lock());
                let v = guard.deref();
                !v.is_empty()
            };

            //only spin up the metrics collector if we have a consumer
            //OR if some actors are sending data we need to consume
            let supervisor = if steady_feature::FEATURE_LEN>0 || we_have_senders {
                supervisor.children(|children| {
                    //we create this child last so we can clone the rx_vec
                    //and capture all the telemetry actors as well
                    let all_tel_rx = self.all_telemetry_rx.clone(); //using Arc here

                    SteadyGraph::configure_for_graph(self, "telemetry-collector"
                                                     , children.with_redundancy(0)
                                                     , move |monitor| {
                            assert_eq!(outgoing.len(), steady_feature::FEATURE_LEN);

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
            };

            supervisor

        }).expect("Telemetry supervisor creation error.");
    }
}


////////////////////////////////////////////////////////////////

pub trait TxDef {
    fn id(&self) -> usize;
    fn batch_limit(&self) -> usize;
    fn set_local_id(& mut self, idx: usize, actor_name: &'static str);

}

impl <T> TxDef for SteadyTx<T> {

    fn id(&self) -> usize {
        self.id
    }
    fn batch_limit(&self) -> usize {
        self.batch_limit
    }
    fn set_local_id(& mut self, idx: usize, actor_name: &'static str) {
        self.local_idx = idx;
        self.actor_name = Some(actor_name);
    }

}

pub trait RxDef {
    fn id(&self) -> usize;
    fn batch_limit(&self) -> usize;
    fn set_local_id(& mut self, idx: usize);
}

impl <T> RxDef for SteadyRx<T> {

    fn id(&self) -> usize {
        self.id
    }
    fn batch_limit(&self) -> usize {
        self.batch_limit
    }

    fn set_local_id(& mut self, idx: usize) {
        self.local_idx = idx;
    }
}


pub trait RxTel : Send + Sync {


    //returns an iterator of usize channel ids for tx channels
    fn tx_ids_iter(&self) -> Box<dyn Iterator<Item=usize> + '_>;
    fn rx_ids_iter(&self) -> Box<dyn Iterator<Item=usize> + '_>;

    fn consume_into(&self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>);

        //NOTE: we will do one dyn call per node every 32ms or so to build the image
    //      we only have 1 impl assuming the compiler will inline this if possible
    // TODO: in the future we could rewrite this to return a future that can be pinned and boxed
 //   fn consume_into(& mut self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>);
    fn biggest_tx_id(&self) -> usize;
    fn biggest_rx_id(&self) -> usize;

}
impl <const RXL: usize, const TXL: usize> RxTel for SteadyTelemetryRx<RXL,TXL> {

    #[inline]
    fn tx_ids_iter(&self) -> Box<dyn Iterator<Item=usize> + '_> {
        Box::new((0..=self.biggest_tx_id()).into_iter())
    }
    #[inline]
    fn rx_ids_iter(&self) -> Box<dyn Iterator<Item=usize> + '_> {
        Box::new((0..=self.biggest_tx_id()).into_iter())
    }


    #[inline]
    fn consume_into(&self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>) {

            //this method can not be async since we need vtable and dyn
        //TODO: revisit later we may be able to return a closure instead

            if let Some(ref rx) = &self.take {
                let mut rx_guard = run!(rx.rx.lock());
                let rx = rx_guard.deref_mut();
                while None != rx.try_take(){    //TODO: not done
                };
            }
            if let Some(ref tx) = &self.send {
                let mut tx_guard = run!(tx.rx.lock());
                let tx = tx_guard.deref_mut();
                while None != tx.try_take(){    //TODO: not done
                };
            }

            //read all the messages available and put the total into the right vec index
            //do for both reads and sends.
            /*run!(async {
            loop {
                let mut count = 0;

                if let Some(ref mut rx) = &self.take {
                        if rx.rx.has_message() {
                            match rx.rx.take_async().await {
                                Ok(a) => {
                                    rx.map.iter()
                                        .zip(a.iter())
                                        .for_each(|(idx, val)| take_target[*idx] += *val as u128);
                                },
                                Err(msg) => {
                                    error!("Unexpected error recv_async: {}",msg);
                                }
                            }
                        } else {
                            count += 1;
                        }
                };

                if let Some(ref mut tx) = &self.send {
                        if tx.rx.has_messages() {
                            match tx.rx.take_async().await {
                                Ok(a) => {
                                    tx.iter().zip(a.iter())
                                        .for_each(|(idx, val)| send_target[*idx] += *val as u128);
                                },
                                Err(msg) => {
                                    error!("Unexpected error recv_async: {}",msg);
                                }
                            }
                        }
                } else {
                    count += 1;
                }
                //leave when both channels are empty
                if 2 == count {
                    break;
                }
            }
            })  */



    }

    fn biggest_tx_id(&self) -> usize {
        if let Some(tx) = &self.send {
            tx.map.iter().max().unwrap_or(&0).clone()
        } else {
            0
        }

    }

    fn biggest_rx_id(&self) -> usize {
        if let Some(tx) = &self.take {
            tx.map.iter().max().unwrap_or(&0).clone()
        } else {
            0
        }
    }
}


impl<T> SteadyRx<T> {
    fn new(id: usize, receiver: InternalReceiver<T>,cap: usize, label: &'static [& 'static str]) -> Self {
         SteadyRx { id, batch_limit: cap, rx: receiver, label, local_idx: usize::MAX  }
    }

    #[inline]
    pub async fn take_async(& mut self) -> Result<T,String> {
         match self.rx.pop().await {
            Some(a) => {Ok(a)}
            None => {Err("Producer is dropped".to_string())}
        }
    }

    #[inline]
    pub fn try_take(& mut self) -> Option<T> {
        self.rx.try_pop()
    }

    #[inline]
    pub fn try_peek(&self) -> Option<&T> {
        self.rx.first()
    }

    //this can and should be in-line when possible, hint to compiler
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }


}
impl<T> SteadyTx<T> {
    fn new(id: usize, sender: InternalSender<T>,cap: usize, label: &'static [& 'static str], actor_name: Option<&'static str>) -> Self {
        SteadyTx {actor_name, id, batch_limit: cap, tx: sender, label, local_idx: usize::MAX }
    }



    #[inline]
    pub fn try_send(& mut self, msg: T) -> Result<(), T> {
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())}
            Err(m) => {Err(m)}
        }
    }

    #[inline]
    pub async fn send_async(& mut self, msg: T) -> Result<(), T> {
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())},
            Err(msg) => {
                error!("full channel detected data_approval tx actor: {:?} labels: {:?} capacity:{:?} type:{} "
                , self.actor_name, self.label, self.batch_limit, type_name::<T>());
                //here we will await until there is room in the channel
                self.tx.push(msg).await
            }
        }
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.tx.is_full()
    }


}
////////////////////////////////////////////
////////////////////////////////////////////

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use async_std::test;
    use lazy_static::lazy_static;
    use std::sync::Once;
    use crate::steady_util::steady_util;


    lazy_static! {
            static ref INIT: Once = Once::new();
    }

    pub(crate) fn initialize_logger() {
        INIT.call_once(|| {
            if let Err(e) = steady_util::steady_logging_init("info") {
                print!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
            }
        });
    }

    //this is my unit test for relay_stats_tx_custom
    #[test]
    async fn test_relay_stats_tx_rx_custom() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_string, rx_string) = graph.new_channel(8,&[]);

        let monitor = graph.new_test_monitor("test");
        let mut rx_string_guard = guard!(rx_string);
        let mut tx_string_guard = guard!(tx_string);

        let rxd: &mut SteadyRx<String> = ref_mut!(rx_string_guard);
        let txd: &mut SteadyTx<String> = ref_mut!(tx_string_guard);

        let mut monitor = monitor.init_stats(&mut[rxd], &mut[txd]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string()).await;
            count += 1;
        }

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_idx], threshold);
        }
        monitor.relay_stats_tx_set_custom_batch_limit(txd, threshold);
        monitor.relay_stats_batch().await;

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_idx], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], threshold);
        }
        monitor.relay_stats_rx_set_custom_batch_limit(rxd, threshold);
        monitor.relay_stats_batch().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], 0);
        }
    }


    #[test]
    async fn test_relay_stats_tx_rx_batch() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let monitor = graph.new_test_monitor("test");

        let (tx_string,rx_string) = graph.new_channel(5,&[]);

        let mut rx_string_guard = guard!(rx_string);
        let mut tx_string_guard = guard!(tx_string);

        let rxd: &mut SteadyRx<String> = ref_mut!(rx_string_guard);
        let txd: &mut SteadyTx<String> = ref_mut!(tx_string_guard);

        let mut monitor = monitor.init_stats(&mut[rxd], &mut[txd]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string()).await;
            count += 1;
            if let Some(ref mut tx) = monitor.telemetry_send_tx {
                assert_eq!(tx.count[txd.local_idx], count);
            }
            monitor.relay_stats_batch().await;
        }

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_idx], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }
        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], threshold);
        }

        monitor.relay_stats_batch().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], 0);
        }

    }
}