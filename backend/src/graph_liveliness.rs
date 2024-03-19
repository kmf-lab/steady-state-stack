use crate::util;
use std::ops::DerefMut;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicUsize};
use std::time::{Duration, Instant};
use futures::lock::Mutex;
use std::process::exit;
use log::{error, warn};
use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use futures::channel::oneshot;
use futures_timer::Delay;

use crate::actor_builder::ActorBuilder;
use crate::{EdgeSimulationDirector, Graph, telemetry};
use crate::channel_builder::ChannelBuilder;
use crate::config::TELEMETRY_PRODUCTION_RATE_MS;

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum GraphLivelinessState {
    Building,
    Running,
    StopRequested, //all actors are voting or changing their vote
    Stopped,
    StoppedUncleanly,
}

#[derive(Default)]
pub struct ShutdownVote {
    pub(crate) ident: ActorIdentity,
    pub(crate) in_favor: bool
}

pub struct GraphLiveliness {
    pub(crate) voters: Arc<AtomicUsize>,
    pub(crate) state: GraphLivelinessState,
    pub(crate) votes: Arc<Vec<Mutex<ShutdownVote>>>,

    pub(crate) one_shot_shutdown: Arc<Option<Vec<oneshot::Sender<()>>>>,
    //NOTE: if actor calls await after shutdown is started then
    //      it may be an unclean shutdown based on timeout.
    //      the await block should be skipped if we are shutting down.??


}



pub(crate) struct WaitWhileRunningFuture {
    shared_state: Arc<RwLock<GraphLiveliness>>,
}
impl WaitWhileRunningFuture {
    pub(crate) fn new(shared_state: Arc<RwLock<GraphLiveliness>>) -> Self {
        Self { shared_state }
    }
}
impl Future for WaitWhileRunningFuture {
    type Output = Result<(), ()>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let self_mut = self.get_mut();

        match self_mut.shared_state.read() {
            Ok(read_guard) => {
                if read_guard.is_in_state(&[GraphLivelinessState::Running]) {
                    Poll::Pending
                } else {
                    Poll::Ready(Ok(()))
                }
            }
            Err(_) => Poll::Pending,
        }
    }
}


impl GraphLiveliness {
    // this is inside a RWLock and
    // returned from LocalMonitor and SteadyContext

    pub(crate) fn new(actors_count: Arc<AtomicUsize>) -> Self {
        GraphLiveliness {
            voters: actors_count,
            state: GraphLivelinessState::Building,
            votes: Arc::new(Vec::new()),
            one_shot_shutdown: Arc::new(Some(Vec::new())),
        }
    }

   pub fn request_shutdown(&mut self) {
       if self.state.eq(&GraphLivelinessState::Running) {
           let voters = self.voters.load(std::sync::atomic::Ordering::SeqCst);

           //print new ballots for this new election
           let votes:Vec<Mutex<ShutdownVote>> = (0..voters)
                             .map(|_| Mutex::new(ShutdownVote::default()) )
                             .collect();
           self.votes = Arc::new(votes);
//TODO: common asncy mthods. on context znd monitor

           //trigger all actors to vote now.
           self.state = GraphLivelinessState::StopRequested;

           if let Some(option) = Arc::get_mut(&mut self.one_shot_shutdown) {
               if let Some(mut shots) = option.take() {
                   while !shots.is_empty() {
                       shots.pop().map(|f| f.send(()));
                   }
               }
           }
       }
   }

    pub fn check_is_stopped(&self, now:Instant, timeout:Duration) -> Option<GraphLivelinessState> {
        assert_eq!(self.state, GraphLivelinessState::StopRequested);
        let is_unanimous = self.votes.iter().all(|f| {
            bastion::run!(
                            async {
                                f.lock().await.deref_mut().in_favor
                            }
                        )
        });

        if is_unanimous {
            Some(GraphLivelinessState::Stopped)
        } else {
            //not unanimous but we are in stop requested state
            if now.elapsed() > timeout {
                Some(GraphLivelinessState::StoppedUncleanly)
            } else {
                None
            }
        }
    }


    pub fn is_in_state(&self, matches: &[GraphLivelinessState]) -> bool {
        matches.iter().any(|f| f.eq(&self.state))
    }

    pub fn is_running(&self, ident: ActorIdentity, accept_fn: &mut dyn FnMut() -> bool) -> bool {
        match self.state {
            GraphLivelinessState::Running => { true }
            GraphLivelinessState::StopRequested => {
                 bastion::run! {
                    let mut guard =self.votes[ident.id].lock().await;
                    let vote = guard.deref_mut();
                    vote.ident = ident; //signature it is me
                    vote.in_favor = accept_fn();
                    !vote.in_favor //return the opposite to keep running when we vote no
                }
            }
            GraphLivelinessState::Building =>  { true }
            GraphLivelinessState::Stopped =>  { false }
            GraphLivelinessState::StoppedUncleanly =>  { false }
        }
    }



}


#[derive(Clone,Debug,Default,Copy,PartialEq,Eq,Hash)]
pub struct ActorIdentity {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
}




impl Graph {

    pub fn actor_builder(&mut self) -> ActorBuilder{
        crate::ActorBuilder::new(self)
    }


    fn enable_fail_fast(&self) {
        std::panic::set_hook(Box::new(|panic_info| {
            let backtrace = std::backtrace::Backtrace::capture();
            // You can log the panic information here if needed
            eprintln!("Application panicked: {}", panic_info);
            eprintln!("Backtrace:\n{:?}", backtrace);
            exit(-1);
        }));
    }


    pub fn edge_simulator(&self, name: & 'static str) -> EdgeSimulationDirector {
        EdgeSimulationDirector::new(self, name)
    }

    /// start the graph, this should be done after building the graph
    pub fn start(&mut self) {

        // if we are not in release mode we will enable fail fast
        // this is most helpful while new code is under development
        if !crate::config::DISABLE_DEBUG_FAIL_FAST {
            #[cfg(debug_assertions)]
            self.enable_fail_fast();
        }

        bastion::Bastion::start(); //start the graph
        match self.runtime_state.write() {
            Ok(mut state) => {
                state.state = GraphLivelinessState::Running;
            }
            Err(e) => {
                error!("failed to start graph: {:?}", e);
            }
        }
    }


    pub fn stop(&mut self) {
        match self.runtime_state.write() {
            Ok(mut a) => {
                a.request_shutdown();
            }
            Err(b) => {
                error!("failed to request stop of graph: {:?}", b);
            }
        }

    }


    pub fn block_until_stopped(self, clean_shutdown_timeout: Duration) {

        let now = Instant::now();
        //duration is not allowed to be less than 3 frames of telemetry
        //this ensures with safety that all actors had an opportunity to
        //raise objections and delay the stop. we just take the max of both
        //durations and do not error or panic since we are shutting down
        let timeout = clean_shutdown_timeout.max(
            Duration::from_millis(3 * crate::config::TELEMETRY_PRODUCTION_RATE_MS as u64));

        //wait for either the timeout or the state to be Stopped
        //while try lock then yield and do until time has passed
        //if we are stopped we will return immediately
        loop {
            //yield to other threads as we are trying to stop
            //all the running actors need to vote
            bastion::run!(util::async_yield_now());
            //now check the lock
            let is_stopped = match self.runtime_state.read() {
                                Ok(state) => {
                                    state.check_is_stopped(now, timeout)
                                }
                                Err(e) => {
                                    error!("failed to read liveliness of graph: {:?}", e);
                                    None
                                }
                            };
            if let Some(shutdown) = is_stopped {
                match self.runtime_state.write() {
                    Ok(mut state) => {
                        state.state = shutdown;

                        if state.state.eq(&GraphLivelinessState::StoppedUncleanly) {
                            warn!("graph stopped uncleanly");
                        }
                    }
                    Err(e) => {
                        error!("failed to request stop of graph: {:?}", e);
                    }
                }
                break;
            } else {
                //allow bastion to process other work while we wait one frame
                bastion::run!(Delay::new(Duration::from_millis(TELEMETRY_PRODUCTION_RATE_MS as u64)));
            }
        }
        bastion::Bastion::kill();

   }



    /// create a new graph for the application typically done in main
    pub fn new<A: Any+Send+Sync>(args: A) -> Graph {
        let channel_count = Arc::new(AtomicUsize::new(0));
        let mut result = Graph {
            args: Arc::new(Box::new(args)),
            channel_count: channel_count.clone(),
            monitor_count: Arc::new(AtomicUsize::new(0)), //this is the count of all monitors
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())), //this is all telemetry receivers
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(channel_count))),
            oneshot_shutdown_vec: Arc::new(Mutex::new(Vec::new())),
        };
        //this is based on features in the config
        telemetry::setup::build_optional_telemetry_graph(&mut result);
        result
    }

    pub fn channel_builder(&mut self) -> ChannelBuilder {
        ChannelBuilder::new(self.channel_count.clone(),self.oneshot_shutdown_vec.clone())
    }

}
