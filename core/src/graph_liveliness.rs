use crate::{abstract_executor, config, SteadyContext, write_warning_to_console};
use std::ops::{ Sub};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use futures::lock::Mutex;
use std::process::exit;
use log::{error, log_enabled, warn};
use std::any::Any;
use std::backtrace::{Backtrace, BacktraceStatus};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{ AtomicUsize};
use std::task::{Context, Poll};
use std::thread;
use colored::Colorize;
use futures::channel::oneshot;
use futures::channel::oneshot::Sender;
use futures_util::lock::{MutexGuard, MutexLockFuture};
use nuclei::config::IoUringConfiguration;
use crate::actor_builder::ActorBuilder;
use crate::telemetry;
use crate::channel_builder::ChannelBuilder;
use crate::config::*;
use crate::graph_testing::SideChannelHub;
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum GraphLivelinessState {
    Building,
    Running,
    StopRequested, //all actors are voting or changing their vote
    Stopped,
    StoppedUncleanly,
}

#[derive(Default,Clone)]
pub struct ShutdownVote {
    pub(crate) ident: Option<ActorIdentity>,
    pub(crate) in_favor: bool
}

pub struct GraphLiveliness {
    pub(crate) voters: Arc<AtomicUsize>,
    pub(crate) state: GraphLivelinessState,
    pub(crate) votes: Arc<Box<[Mutex<ShutdownVote>]>>,
    pub(crate) vote_in_favor_total: AtomicUsize,
    pub(crate) shutdown_one_shot_vec: Arc<Mutex<Vec<Sender<()>>>>,
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

    pub(crate) fn new(actors_count: Arc<AtomicUsize>, one_shot_shutdown: Arc<Mutex<Vec<Sender<()>>>>) -> Self {
        GraphLiveliness {
            voters: actors_count,
            state: GraphLivelinessState::Building,
            votes: Arc::new(Box::new([])),
            vote_in_favor_total: AtomicUsize::new(0),
            shutdown_one_shot_vec: one_shot_shutdown
        }
    }

    pub(crate) fn building_to_running(&mut self) {
        if self.state.eq(&GraphLivelinessState::Building) {
            self.state = GraphLivelinessState::Running;
            //trace!("now running");
        } else {
            error!("unexpected state {:?}",self.state);
        }
    }

   pub fn request_shutdown(&mut self) {
       if self.state.eq(&GraphLivelinessState::Running) {
           let voters = self.voters.load(std::sync::atomic::Ordering::SeqCst);

           //print new ballots for this new election
           let votes:Vec<Mutex<ShutdownVote>> = (0..voters)
                             .map(|_| Mutex::new(ShutdownVote::default()) )
                             .collect();
           self.votes = Arc::new(votes.into_boxed_slice());
           self.vote_in_favor_total.store(0,std::sync::atomic::Ordering::SeqCst); //redundant but safe for clarity

           //trigger all actors to vote now.
           self.state = GraphLivelinessState::StopRequested;

           let local_oss = self.shutdown_one_shot_vec.clone();
           abstract_executor::block_on(async move {
               let mut one_shots:MutexGuard<Vec<Sender<_>>> = local_oss.lock().await;
               while let Some(f) = one_shots.pop() {
                   f.send(()).expect("oops");
               }
           });

       }
   }

    pub fn check_is_stopped(&self, now:Instant, timeout:Duration) -> Option<GraphLivelinessState> {
        if self.is_in_state(&[GraphLivelinessState::StopRequested, GraphLivelinessState::Stopped, GraphLivelinessState::StoppedUncleanly]) {
            //if the count in favor is the same as the count of total votes then we have unanimous agreement
            if self.vote_in_favor_total.load(std::sync::atomic::Ordering::SeqCst) == self.votes.len() {
                Some(GraphLivelinessState::Stopped)
            } else {
                //not unanimous but we are in stop requested state
                if now.elapsed() > timeout {
                    Some(GraphLivelinessState::StoppedUncleanly)
                } else {
                    None
                }
            }
        } else {
            None
        }
    }


    pub fn is_in_state(&self, matches: &[GraphLivelinessState]) -> bool {
        matches.iter().any(|f| f.eq(&self.state))
    }

    pub fn is_running(&self, ident: ActorIdentity, accept_fn: &mut dyn FnMut() -> bool) -> bool {
        match self.state {
            GraphLivelinessState::Building => {
                //allow run to start but also let the other startup if needed.
                thread::yield_now();
                true
            }
            GraphLivelinessState::Running => { true }
            GraphLivelinessState::StopRequested => {
                let in_favor = accept_fn();
                let my_ballot = &self.votes[ident.id];
                if let Some(mut vote) = my_ballot.try_lock() {
                    vote.ident = Some(ident); //signature it is me
                    if in_favor && !vote.in_favor {
                        //safe total where we know this can only be done once for each
                        self.vote_in_favor_total.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    vote.in_favor = in_favor;
                    drop(vote);
                    !in_favor //return the opposite to keep running when we vote no
                } else {
                    //NOTE: this may be the reader not a voter: TODO: fix.
                    error!("voting integrity error, someone else has my ballot {:?} in_favor of shutdown: {:?}",ident,in_favor);
                    true //if we can't vote we oppose the shutdown by continuing to run
                }
            }
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

pub enum ProactorConfig { //NOTE: these descriptions are from the nuclei comments
    /// Standard way to use IO_URING. No polling, purely IRQ awaken IO completion.
    /// This is a normal way to process IO, mind that with this approach
    /// actual completion time != userland reception of completion. Throughput is low compared
    /// to all the other config alternatives.
    /// NOTE: this is the default and the lowest CPU solution.
    InterruptDriven,
    /// Kernel poll only version of IO_URING, where it is suitable for high traffic environments.
    /// This version won't allow aggressive polling on completion queue(CQ).
    KernelPollDriven,
    /// Low Latency Driven version of IO_URING, where it is suitable for high traffic environments.
    /// High throughput low latency solution where it consumes a lot of resources.
    LowLatencyDriven,
    /// IOPOLL enabled ring configuration for operating on files with low-latency.
    IoPoll,
}




pub struct Graph  {
    pub(crate) args: Arc<Box<dyn Any+Send+Sync>>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) monitor_count: Arc<AtomicUsize>,
    //used by collector but could grow if we get new actors at runtime
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    pub(crate) oneshot_startup_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    pub(crate) backplane: Arc<Mutex<Option<SideChannelHub>>>, //only used in testing
    pub(crate) noise_threshold: Instant,
    pub(crate) block_fail_fast: bool,
    pub(crate) iouring_queue_length: u32,
    pub(crate) enable_io_driver: bool,
    pub(crate) proactor_config: ProactorConfig,
    pub(crate) telemetry_production_rate_ms: u64,

}

impl Graph {

    /// if you are sure you have no io and will not add any ioring io this can be disabled.
    /// do not call unless you are sure this will not be needed.
    pub fn disable_io_driver(&mut self) {
        self.enable_io_driver = false;
    }

    pub fn iouring_queue_length(&self) -> u32 {
        self.iouring_queue_length
    }

    pub fn set_iouring_queue_length(&mut self, iouring_queue_length: u32) {
        self.iouring_queue_length = iouring_queue_length;
    }

    pub fn set_proactor_config(&mut self, proactor_config: ProactorConfig) {
        self.proactor_config = proactor_config;
    }


    /// needed for testing only, this monitor assumes we are running without a full graph
    /// and will not be used in production
    pub fn new_test_monitor(&self, name: & 'static str ) -> SteadyContext
    {

        // assert that we are NOT in release mode
        assert!(cfg!(debug_assertions), "This function is only for testing");

        let channel_count    = self.channel_count.clone();
        let all_telemetry_rx = self.all_telemetry_rx.clone();

        let oneshot_shutdown = {
            let (send_shutdown_notice_to_periodic_wait, oneshot_shutdown) = oneshot::channel();
            let local_vec = self.oneshot_shutdown_vec.clone();
            abstract_executor::block_on(
                async move {
                    local_vec.lock().await.push(send_shutdown_notice_to_periodic_wait);
                }
            );
            oneshot_shutdown
        };
        let oneshot_shutdown = Arc::new(Mutex::new(oneshot_shutdown));
        let now = Instant::now();

        SteadyContext {
            channel_count,
            ident: ActorIdentity{name, id: self.monitor_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst)},
            args: self.args.clone(),
            is_in_graph: false, //this is key, we are not running in a graph by design
            actor_metadata: Arc::new(ActorMetaData::default()),
            all_telemetry_rx,
            runtime_state: self.runtime_state.clone(),
            instance_id: 0,
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown,
            last_periodic_wait: Default::default(),
            actor_start_time: now,
            node_tx_rx: None,
            frame_rate_ms: self.telemetry_production_rate_ms
        }
    }

    pub fn actor_builder(&mut self) -> ActorBuilder{
        crate::ActorBuilder::new(self)
    }


    fn enable_fail_fast(&self) {
        //runtime disable of fail fast so we can unit test the recovery of panics
        //where normally in a test condition we would want to fail fast
        if !self.block_fail_fast {
            std::panic::set_hook(Box::new(|panic_info| {
                // Capture the backtrace
                let backtrace = Backtrace::capture();
                // Pretty print the backtrace if it's captured
                match backtrace.status() {
                    BacktraceStatus::Captured => {

                        eprintln!("{:?}", panic_info);

                        eprintln!("{:?}", backtrace.status());


                        let backtrace_str = format!("{:#?}", backtrace);

                        let mut is_first_unknown_line = true;
                            for line in backtrace_str.lines() {
                                if line.contains("steady_state") || line.contains("Backtrace [") || line.contains("nuclei::proactor::") {
                                    //avoid printing our line we used for generating the stack trace
                                    if !line.contains("graph_liveliness::<impl steady_state::Graph>::enable_fail_fast") {
                                        //make the steady_steady state lines green
                                        eprintln!("{}", line.green());
                                        //stop early since we have the bottom of the actor
                                        if line.contains("steady_state::actor_builder::launch_actor") {
                                            eprintln!("{}", "]".green());
                                            break;
                                        }
                                    }
                                } else {
                                    //if is first it is the call to this function and not something to review.
                                    if is_first_unknown_line || line.contains("::panicking::") || line.contains("::backtrace::") || line.contains("begin_unwind")  {
                                        if log_enabled!(log::Level::Trace) {
                                            eprintln!("{}", line.blue()); //no need to see the panic logic most of the time unless we are debugging
                                        }
                                    } else {
                                        eprintln!("{}", line);
                                    }
                                    is_first_unknown_line = false;
                                }
                            }
                        }

                    _ => {
                        eprintln!("Backtrace could not be captured: {}",panic_info);
                    }
                }
                exit(-1);
            }));
        }
    }


    pub fn sidechannel_director(&self) -> MutexLockFuture<'_, Option<SideChannelHub>> {
        self.backplane.lock()
    }


    /// start the graph, this should be done after building the graph
    pub fn start(&mut self) {
        // error!("start called");

        // if we are not in release mode we will enable fail fast
        // this is most helpful while new code is under development
        if !crate::config::DISABLE_DEBUG_FAIL_FAST {
            #[cfg(debug_assertions)]
            self.enable_fail_fast();
        }

        let nuclei_config = match self.proactor_config {
            ProactorConfig::InterruptDriven => {IoUringConfiguration::interrupt_driven(self.iouring_queue_length)}
            ProactorConfig::KernelPollDriven => {IoUringConfiguration::kernel_poll_only(self.iouring_queue_length)}
            ProactorConfig::LowLatencyDriven => {IoUringConfiguration::low_latency_driven(self.iouring_queue_length)}
            ProactorConfig::IoPoll => {IoUringConfiguration::io_poll(self.iouring_queue_length)}
        };

        abstract_executor::init(self.enable_io_driver, nuclei_config);

        //trace!("start was called");
        //everything has been scheduled and running so change the state
        match self.runtime_state.write() {
            Ok(mut state) => {
                //error!("to running");
                state.building_to_running();
               // error!("we are now  running");
                let v = self.oneshot_startup_vec.clone();
                abstract_executor::block_on( async move {
                    let mut one_shots:MutexGuard<Vec<Sender<_>>> = v.lock().await;
                    while let Some(sender) = one_shots.pop() {
                        if let Err(e) =  sender.send(()) {
                                error!("failed to send startup signal: {:?}",e);
                        }
                    }
                });
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

        if let Some(wait_on) = match self.runtime_state.write() {
            Ok(state) => {

                if state.is_in_state(&[GraphLivelinessState::Running, GraphLivelinessState::Building]) {
                    let (tx, rx) = futures::channel::oneshot::channel();
                    let v = state.shutdown_one_shot_vec.clone();
                    abstract_executor::block_on( async move {
                       v.lock().await.push(tx);
                    });
                    Some(rx)
                } else {
                    None
                }
            },
            Err(e) => {
                error!("failed to request stop of graph: {:?}", e);
                None
            }
        } {
            //if we are Running or Building then we block here until shutdown is started.
            let _ = abstract_executor::block_on(wait_on);
        }


        let now = Instant::now();
        //duration is not allowed to be less than 3 frames of telemetry
        //this ensures with safety that all actors had an opportunity to
        //raise objections and delay the stop. we just take the max of both
        //durations and do not error or panic since we are shutting down
        let timeout = clean_shutdown_timeout.max(
            Duration::from_millis(3 * self.telemetry_production_rate_ms as u64));

        //wait for either the timeout or the state to be Stopped
        //while try lock then yield and do until time has passed
        //if we are stopped we will return immediately
        loop {
            let is_stopped = {match self.runtime_state.read() {
                        Ok(state) => {
                            state.check_is_stopped(now, timeout)
                        }
                        Err(e) => {
                            error!("failed to read liveliness of graph: {:?}", e);
                            None
                        }
                    }};
            if let Some(shutdown) = is_stopped {
                match self.runtime_state.write() {
                    Ok(mut state) => {
                        state.state = shutdown;

                        if state.state.eq(&GraphLivelinessState::StoppedUncleanly) {
                            warn!("voter log: (approved votes at the top, total:{})",state.votes.len());
                            let mut voters = state.votes.iter()
                                .map(|f| f.try_lock().map(|v| v.clone()) )
                                .collect::<Vec<_>>();

                            // You can sort or prioritize the votes as needed here
                            voters.sort_by_key(|voter| ! voter.as_ref().map_or(false, |f| f.in_favor)); // This will put `true` (in favor) votes first

                            // Now iterate over the sorted voters and log the results
                            voters.iter().for_each(|voter| {
                                warn!("Voted: {:?} Ident: {:?}", voter.as_ref().map_or(false, |f| f.in_favor), voter.as_ref().map_or(Default::default(), |f| f.ident));
                            });
                            warn!("graph stopped uncleanly");
                        }
                    }
                    Err(e) => {
                        error!("failed to request stop of graph: {:?}", e);
                    }
                }
                break;
            } else {
                thread::sleep(Duration::from_millis(self.telemetry_production_rate_ms));
            }
        }

   }



    /// create a new graph for the application typically done in main
    pub fn new<A: Any+Send+Sync>(args: A) -> Graph {
        let block_fail_fast = false;
        Self::internal_new(args, block_fail_fast)
    }

    /// used for normal create and unit test create. unit tests must block the fail fast for panic testing
    pub(crate) fn internal_new<A: Any + Send + Sync>(args: A
                               , block_fail_fast: bool

                                ) -> Graph {
        let channel_count = Arc::new(AtomicUsize::new(0));
        let monitor_count = Arc::new(AtomicUsize::new(0));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
        let oneshot_startup_vec = Arc::new(Mutex::new(Vec::new()));

        //only used for testing but this backplane is here to support
        //dynamic type message sending to and from all nodes for coordination of testing
        let backplane: Option<SideChannelHub> = None;
        #[cfg(test)]
        let backplane = Some(SideChannelHub::new());


        let mut result = Graph {
            args: Arc::new(Box::new(args)),
            channel_count: channel_count.clone(),
            monitor_count: monitor_count.clone(), //this is the count of all monitors
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())), //this is all telemetry receivers
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(monitor_count, oneshot_shutdown_vec.clone()))),
            oneshot_shutdown_vec,
            oneshot_startup_vec,
            backplane: Arc::new(Mutex::new(backplane)),
            noise_threshold: Instant::now().sub(Duration::from_secs(config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64)),
            block_fail_fast,
            iouring_queue_length: 1<<5,
            enable_io_driver: true,
            proactor_config: ProactorConfig::InterruptDriven,
            telemetry_production_rate_ms: 40, //TODO: set default somewhere.
        };
        //this is based on features in the config
        telemetry::setup::build_optional_telemetry_graph(&mut result);
        result
    }

    pub fn channel_builder(&mut self) -> ChannelBuilder {
        ChannelBuilder::new(self.channel_count.clone(),self.oneshot_shutdown_vec.clone(),self.noise_threshold,self.telemetry_production_rate_ms)
    }

}