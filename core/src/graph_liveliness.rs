//! This module provides core functionalities for the SteadyState project, including the
//! graph and graph liveliness components. The graph manages the execution of actors,
//! and the liveliness state handles the shutdown process and state transitions.

use crate::{abstract_executor, steady_config, SteadyContext, util};
use std::ops::Sub;
use std::sync::{Arc};
use parking_lot::{RwLock,RwLockWriteGuard};
use std::time::{Duration, Instant};
use futures::lock::Mutex;
use std::process::exit;
#[allow(unused_imports)]
use log::{error, info, log_enabled, trace, warn};
use std::any::Any;
use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::{io, thread};
use std::io::Write;
use futures::channel::oneshot;
use futures::channel::oneshot::{Sender};

use futures_util::lock::{MutexGuard, MutexLockFuture};
use nuclei::config::IoUringConfiguration;
use crate::actor_builder::ActorBuilder;
use crate::telemetry;
use crate::channel_builder::ChannelBuilder;
use crate::graph_testing::SideChannelHub;
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;

/// Represents the state of graph liveliness in the Steady State framework.
///
/// The `GraphLivelinessState` enum is used to indicate the current state of the graph of actors.
/// This state can change as the graph is built, run, or stopped.
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum GraphLivelinessState {
    /// The graph is currently being built.
    ///
    /// In this state, actors are being added to the graph and it is not yet ready to run.
    Building,

    /// The graph is currently running.
    ///
    /// In this state, all actors in the graph are executing their tasks concurrently.
    Running,

    /// A stop request has been issued, and all actors are voting or changing their vote on the stop request.
    ///
    /// In this state, the graph is in the process of shutting down, but not all actors have stopped yet.
    StopRequested,

    /// The graph has been stopped.
    ///
    /// In this state, all actors have ceased execution and the graph is no longer running.
    Stopped,

    /// The graph has stopped, but not all actors have stopped cleanly.
    ///
    /// In this state, the graph encountered issues during shutdown, resulting in some actors not stopping as expected.
    StoppedUncleanly,
}


/// Represents a vote for shutdown by an actor.
#[derive(Default, Clone)]
pub struct ShutdownVote {
    pub(crate) id: usize,
    pub(crate) ident: Option<ActorIdentity>,
    pub(crate) in_favor: bool,
}

/// Manages the liveliness state of the graph and handles the shutdown voting process.
pub struct GraphLiveliness {
    pub(crate) registered_voters: Vec<Option<ActorIdentity>>,
    pub(crate) state: GraphLivelinessState,
    pub(crate) votes: Arc<Box<[Mutex<ShutdownVote>]>>,
    pub(crate) vote_in_favor_total: AtomicUsize,
    pub(crate) shutdown_one_shot_vec: Arc<Mutex<Vec< Sender<()> >>>,
    pub(crate) registered_voter_count: AtomicUsize,
    pub(crate) actors_count: Arc<AtomicUsize>,
}

/// A future that waits while the graph is running.
pub(crate) struct WaitWhileRunningFuture {
    shared_state: Arc<RwLock<GraphLiveliness>>,
}

impl WaitWhileRunningFuture {
    /// Creates a new `WaitWhileRunningFuture`.
    pub(crate) fn new(shared_state: Arc<RwLock<GraphLiveliness>>) -> Self {
        Self { shared_state }
    }
}

impl Future for WaitWhileRunningFuture {
    type Output = Result<(), ()>;

    /// Polls the future to determine if the graph is still running.
    ///
    /// # Arguments
    ///
    /// * `self` - A pinned reference to the future.
    /// * `cx` - The task context used for waking up the task.
    ///
    /// # Returns
    ///
    /// * `Poll::Pending` - If the graph is still running.
    /// * `Poll::Ready(Ok(()))` - If the graph is no longer running.
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let self_mut = self.get_mut();

        let read_guard = self_mut.shared_state.read();
        if read_guard.is_in_state(&[GraphLivelinessState::Running]) {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }
}

impl GraphLiveliness {
    /// Creates a new `GraphLiveliness` instance.
    ///
    /// # Arguments
    ///
    /// * `actors_count` - The number of actors in the graph.
    /// * `one_shot_shutdown` - A one-shot channel for shutdown notifications.
    ///
    /// # Returns
    ///
    /// A new `GraphLiveliness` instance.
    pub(crate) fn new(one_shot_shutdown: Arc<Mutex<Vec< Sender<()> >>>
                      , actors_count: Arc<AtomicUsize>) -> Self {

        GraphLiveliness {
            actors_count,
            registered_voter_count: AtomicUsize::new(0),
            registered_voters: Vec::new(),
            state: GraphLivelinessState::Building,
            votes: Arc::new(Box::new([])),
            vote_in_favor_total: AtomicUsize::new(0),
            shutdown_one_shot_vec: one_shot_shutdown,
        }
    }

    /// Transitions the state from building to running.
    ///
    /// # Panics
    ///
    /// Panics if the current state is not `GraphLivelinessState::Building`.
    pub(crate) fn building_to_running(&mut self) {
        if self.state.eq(&GraphLivelinessState::Building) {
            self.state = GraphLivelinessState::Running;
        } else {
            error!("unexpected state {:?}", self.state);
        }
    }

    pub fn register_voter(&mut self, ident: ActorIdentity) {
            if ident.id >= self.registered_voters.len() {
                self.registered_voters.resize(ident.id + 1, None);
            }
            trace!("register voter {:?} of expected count: {:?}",ident,self.actors_count.load(Ordering::SeqCst));

            if self.registered_voters[ident.id].is_none() {
                self.registered_voter_count.fetch_add(1, Ordering::SeqCst);
            }
            self.registered_voters[ident.id] = Some(ident);

    }

    pub fn wait_for_registrations(&mut self, timeout: Duration) {
        if self.actors_count.load(Ordering::SeqCst)>0 {
           //will rarely take the while loop since all the actors have had a while to start on threads
           //this is only done once on startup an is not async by design.
            trace!("waiting for actors to register: {:?} vs {:?}", self.registered_voter_count.load(Ordering::SeqCst), self.actors_count.load(Ordering::SeqCst));
            let start = Instant::now();
            while self.registered_voter_count.load(Ordering::SeqCst) < self.actors_count.load(Ordering::SeqCst) {
                trace!(" waiting for actors to register: {:?} vs {:?}", self.registered_voter_count.load(Ordering::SeqCst), self.actors_count.load(Ordering::SeqCst));
                let elapsed = start.elapsed();
                if elapsed > timeout {
                    error!("timeout on startup, not all actors registered: {:?} vs {:?}", self.registered_voter_count.load(Ordering::SeqCst), self.actors_count.load(Ordering::SeqCst));
                    break;
                }
                thread::sleep(Duration::from_millis(40));
            }

        } else {
            trace!("no actors to wait for");
        }
        trace!("changed to running state");
        self.building_to_running();
    }

    /// Requests a shutdown of the graph.
    pub fn request_shutdown(&mut self) {
        if self.state.eq(&GraphLivelinessState::Running) {
            let voters = self.registered_voters.len();

            // Print new ballots for this new election
            let votes: Vec<Mutex<ShutdownVote>> = (0..voters)
                .map(|i| Mutex::new(ShutdownVote {
                                    id: i,
                                    ident: None,
                                    in_favor: false,
                }))
                .collect();
            self.votes = Arc::new(votes.into_boxed_slice());
            self.vote_in_favor_total.store(0, std::sync::atomic::Ordering::SeqCst); // Redundant but safe for clarity

            // Trigger all actors to vote now
            self.state = GraphLivelinessState::StopRequested;

            let local_oss = self.shutdown_one_shot_vec.clone();
            abstract_executor::block_on(async move {
                let mut one_shots: MutexGuard<Vec<Sender<_>>> = local_oss.lock().await;
                while let Some(f) = one_shots.pop() {
                    let _ignore = f.send(()); //May already been done but we don't care
                    //Target actor may have already stopped so this is not an error
                }
            });
            //trace!("every actor has had one shot shutdown fired now");
        } else if self.is_in_state(&[GraphLivelinessState::Building]) {
            warn!("request_stop should only be called after start");
        }
        
    }

    /// Checks if the graph is stopped.
    ///
    /// # Arguments
    ///
    /// * `now` - The current time.
    /// * `timeout` - The timeout duration.
    ///
    /// # Returns
    ///
    /// An optional `GraphLivelinessState` indicating the new state.
    pub fn check_is_stopped(&self, now: Instant, timeout: Duration) -> Option<GraphLivelinessState> {
        if self.is_in_state(&[
            GraphLivelinessState::StopRequested,
            GraphLivelinessState::Stopped,
            GraphLivelinessState::StoppedUncleanly,
        ]) {
            // If the count in favor is the same as the count of total votes, then we have unanimous agreement
            if self.vote_in_favor_total.load(std::sync::atomic::Ordering::SeqCst) == self.votes.len() {
                Some(GraphLivelinessState::Stopped)
            } else {
                // Not unanimous but we are in stop requested state
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

    /// Checks if the graph is in one of the specified states.
    ///
    /// # Arguments
    ///
    /// * `matches` - A slice of `GraphLivelinessState` to match against.
    ///
    /// # Returns
    ///
    /// `true` if the graph is in one of the specified states, `false` otherwise.
    pub fn is_in_state(&self, matches: &[GraphLivelinessState]) -> bool {
        let result = matches.iter().any(|f| f.eq(&self.state));
        trace!("is in state called: {:?} looking for {:?} match {:?}", self.state, matches, result);
        result
    }

    /// Checks if an actor is running.
    ///
    /// # Arguments
    ///
    /// * `ident` - The identity of the actor.
    /// * `accept_fn` - A function to determine if the actor accepts the shutdown request.
    ///
    /// # Returns
    ///
    /// Option `true` if the actor is running, `false` otherwise. None if the actor is building.
    pub(crate) fn is_running(&self, ident: ActorIdentity, accept_fn: &mut dyn FnMut() -> bool) -> Option<bool> {
        //warn!("is running called on {:?}", ident);

        match self.state {
            GraphLivelinessState::Building => {
                //  info!("building now: {:?}", ident);
                // Let the other startup if needed.
                thread::yield_now();
                None
            }
            GraphLivelinessState::Running => {
               //info!("running now: {:?}", ident);
                Some(true)
            },
            GraphLivelinessState::StopRequested => {
               //info!("stop requested, voting now: {:?}", ident);

                let in_favor = accept_fn();

                let my_ballot = &self.votes[ident.id];
                if let Some(mut vote) = my_ballot.try_lock() {
                    assert_eq!(vote.id, ident.id);
                    vote.ident = Some(ident); // Signature it is me
                    if in_favor && !vote.in_favor {
                        // Safe total where we know this can only be done once for each
                        self.vote_in_favor_total.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    } else if vote.in_favor {
                        error!("already voted in favor! : {:?} {:?} vs {:?}",ident,in_favor, vote.in_favor);
                    }
                    
                    vote.in_favor = in_favor;
                    drop(vote);
                    Some(!in_favor) // Return the opposite to keep running when we vote no
                } else {
                    // NOTE: this may be the reader not a voter:
                    error!("voting integrity error, someone else has my ballot {:?} in_favor of shutdown: {:?}", ident, in_favor);
                    Some(true) // If we can't vote we oppose the shutdown by continuing to run
                }

            }
            GraphLivelinessState::Stopped => {
               // info!("stopped now: {:?}", ident);
                Some(false)
            },
            GraphLivelinessState::StoppedUncleanly => {
               //info!("stopped unclean now: {:?}", ident);

                Some(false)},
        }
    }
}

/// Represents the identity of an actor.
#[derive(Clone, Default, Copy, PartialEq, Eq, Hash)]
pub struct ActorIdentity {
    pub id: usize,         //unique identifier
    pub label: ActorName,  //for backplane
}
#[derive(Clone, Default, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ActorName {
    pub name: &'static str,    //for backplane
    pub suffix: Option<usize>, //for backplane
}

impl ActorIdentity {
    pub fn new(id: usize, name: &'static str, suffix: Option<usize>) -> Self {
        ActorIdentity {
            id,
            label: ActorName {
                    name,
                    suffix,
            },
        }
    }
}
impl ActorName {
    pub fn new(name: &'static str, suffix: Option<usize>) -> Self {
        ActorName {
            name,
            suffix,
        }
    }
}

impl Debug for ActorIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:?} {}", self.id, self.label.name)?;
        if let Some(suffix) = self.label.suffix {
            write!(f, "-{}", suffix)?;
        }
        Ok(())
    }
}

/// Configuration options for the proactor.
#[derive(Clone, Debug)]
pub enum ProactorConfig {
    /// Standard way to use IO_URING. No polling, purely IRQ awaken IO completion.
    /// This is a normal way to process IO, mind that with this approach
    /// actual completion time != userland reception of completion. Throughput is low compared
    /// to all the other config alternatives.
    /// NOTE: this is the default and the lowest CPU solution.
    InterruptDriven,
    /// Kernel poll only version of IO_URING, where it is suitable for high traffic environments.
    /// This version won't allow aggressive polling on completion queue (CQ).
    KernelPollDriven,
    /// Low Latency Driven version of IO_URING, where it is suitable for high traffic environments.
    /// High throughput low latency solution where it consumes a lot of resources.
    LowLatencyDriven,
    /// IOPOLL enabled ring configuration for operating on files with low-latency.
    IoPoll,
}

#[derive(Clone, Debug)]
pub struct GraphBuilder {
    block_fail_fast: bool,
    telemetry_metric_features: bool,
    enable_io_driver: bool,
    backplane: Option<SideChannelHub>,
    proactor_config: Option<ProactorConfig>,
    iouring_queue_length: u32,
    telemtry_production_rate_ms: u64
    
}

impl Default for GraphBuilder {
    fn default() -> Self {
        GraphBuilder::for_production()
    }
}

impl GraphBuilder {

    /// build the default production or normal graph builder.
    ///
    pub fn for_production() -> Self {
        #[cfg(test)]
        panic!("should not call for_production in tests");
        #[cfg(not(test))]
      GraphBuilder {
          block_fail_fast: true,
          telemetry_metric_features: steady_config::TELEMETRY_SERVER,
          enable_io_driver: true,
          backplane: None,
          proactor_config: Some(ProactorConfig::InterruptDriven),
          iouring_queue_length: 1<<5,
          telemtry_production_rate_ms: 40,
      }
    }

    /// build a default graph builder for running unit tests.  This has backplane communications
    /// enabled for test building.
    ///
    pub fn for_testing() -> Self {
        util::logger::initialize();
        GraphBuilder {
            block_fail_fast: false,
            telemetry_metric_features: false,
            enable_io_driver: true,
            backplane: Some(SideChannelHub::default()),
            proactor_config: Some(ProactorConfig::InterruptDriven),
            iouring_queue_length: 1<<5,
            telemtry_production_rate_ms: 40,
        }
    }


    /// set the iouring queue length
    /// this may be too short by default and when lots of IO is required may need to grow
    ///
    pub fn with_iouring_queue_length(&self, len: u32) -> Self {
        let mut result = self.clone();
        result.iouring_queue_length = len;
        result
    }

    /// set telemetry production frame rate in MS.  For very large graphs it may be necessary
    /// to set this to a larger value to keep up with the volume of data.
    ///
    pub fn with_telemtry_production_rate_ms(&self, ms: u64) -> Self {
        let mut result = self.clone();
        result.telemtry_production_rate_ms = ms;
        result
    }

    /// set the metric features to ensure we can scrape this server from Prometheus
    ///
    pub fn with_telemetry_metric_features(&self, enable: bool) -> Self {
        let mut result = self.clone();
        result.telemetry_metric_features = enable;
        result
    }

    /// consume the graph builder and build the graph for use
    ///
    pub fn build<A: Any + Send + Sync>(self, args: A) -> Graph {
        Graph::internal_new(args
                , self.block_fail_fast
                , self.telemetry_metric_features
                , self.backplane
                , self.proactor_config
                , self.enable_io_driver
                , self.iouring_queue_length
                , self.telemtry_production_rate_ms)
    }
}

/// Manages the execution of actors in the graph.
pub struct Graph {
    pub(crate) args: Arc<Box<dyn Any + Send + Sync>>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) actor_count: Arc<AtomicUsize>,
    pub(crate) thread_lock: Arc<Mutex<()>>,

    // Used by collector but could grow if we get new actors at runtime
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    pub(crate) backplane: Arc<Mutex<Option<SideChannelHub>>>, // Only used in testing
    pub(crate) noise_threshold: Instant,
    pub(crate) block_fail_fast: bool,
    pub(crate) telemetry_production_rate_ms: u64,
}

impl Graph {


    /// Returns the telemetry production rate in milliseconds.
    pub fn telemetry_production_rate_ms(&self) -> u64 {
        self.telemetry_production_rate_ms
    }

     /// Creates a new test monitor.
    ///
    /// This monitor assumes we are running without a full graph and will not be used in production.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the test monitor.
    ///
    /// # Returns
    ///
    /// A new `SteadyContext` for testing.
    ///
    /// # Panics
    ///
    /// Panics if called in release mode.
    pub fn new_test_monitor(&self, name: &'static str) -> SteadyContext {
        // Assert that we are NOT in release mode
        assert!(cfg!(debug_assertions), "This function is only for testing");

        let channel_count = self.channel_count.clone();
        let all_telemetry_rx = self.all_telemetry_rx.clone();

        let oneshot_shutdown_rx = {
            let (send_shutdown_notice_to_periodic_wait, rx) = oneshot::channel();
            let local_vec = self.oneshot_shutdown_vec.clone();
            abstract_executor::block_on(async move {
                local_vec.lock().await.push(send_shutdown_notice_to_periodic_wait);
            });
            rx
        };
        let oneshot_shutdown = Arc::new(Mutex::new(oneshot_shutdown_rx));
        let now = Instant::now();
        
            
        SteadyContext {
            channel_count,
            ident: ActorIdentity::new(usize::MAX, name, None), //no Id this is for testing
            args: self.args.clone(),
            is_in_graph: false, // This is key, we are not running in a graph by design
            actor_metadata: Arc::new(ActorMetaData::default()),
            all_telemetry_rx,
            runtime_state: self.runtime_state.clone(),
            instance_id: 0,
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown,
            last_periodic_wait: Default::default(),
            actor_start_time: now,
            node_tx_rx: None,
            frame_rate_ms: self.telemetry_production_rate_ms, //zero will disable telemetry
        }
    }

    /// Returns an `ActorBuilder` for constructing new actors.
    pub fn actor_builder(&mut self) -> ActorBuilder {
        ActorBuilder::new(self)
    }

    /// Enables fail-fast behavior.
    ///
    /// Runtime disable of fail-fast so we can unit test the recovery of panics
    /// where normally in a test condition we would want to fail fast.
    pub fn enable_fail_fast(&self) {
        // Runtime disable of fail-fast so we can unit test the recovery of panics
        // where normally in a test condition we would want to fail fast
        if !self.block_fail_fast {
            std::panic::set_hook(Box::new(|panic_info| {
                Self::fail_fast_stack_trace(panic_info,&mut io::stderr());
                exit(-1);
            }));
        }
    }

    pub(crate) fn fail_fast_stack_trace<T, W>(panic_info: &T, writer: &mut W)
    where
        T: Debug,
        W: Write,
    {
        // Capture the backtrace
        let backtrace = Backtrace::capture();

        // Write the backtrace information to the given writer
        match backtrace.status() {
            BacktraceStatus::Captured => {
                writeln!(writer, "{:?}", panic_info).unwrap();
                writeln!(writer, "{:?}", backtrace.status()).unwrap();

                let backtrace_str = format!("{:#?}", backtrace);

                for line in backtrace_str.lines() {
                    if line.contains("steady_state") || line.contains("Backtrace [") || line.contains("nuclei::proactor::") {
                        if !line.contains("graph_liveliness::<impl steady_state::Graph>::enable_fail_fast") {
                            writeln!(writer, "{}", line).unwrap(); // Here you'd add color formatting if needed
                            if line.contains("steady_state::actor_builder::launch_actor") {
                                writeln!(writer, "]").unwrap();
                                break;
                            }
                        }
                    } else {
                       // if is_first_unknown_line || line.contains("::panicking::") || line.contains("::backtrace::") || line.contains("begin_unwind") {
                            writeln!(writer, "{}", line).unwrap();
                       // } else {
                       //     writeln!(writer, "{}", line).unwrap();
                       // }
                    }
                }
            }
            _ => {
                writeln!(writer, "Backtrace could not be captured: {:?}", panic_info).unwrap();
            }
        }
    }

    /// Returns a future that locks the side channel hub.
    pub fn sidechannel_director(&self) -> MutexLockFuture<'_, Option<SideChannelHub>> {
        self.backplane.lock()
    }

    /// Starts the graph.
    ///
    /// This should be done after building the graph.
    pub fn start(&mut self) {
        self.start_with_timeout(Duration::from_secs(40));
    }

    /// Starts the graph.
    ///
    /// This should be done after building the graph.
    pub fn start_with_timeout(&mut self, duration:Duration) -> bool {
        // error!("start called");

        // If we are not in release mode we will enable fail-fast
        // This is most helpful while new code is under development
        if !crate::steady_config::DISABLE_DEBUG_FAIL_FAST {
            #[cfg(debug_assertions)]
            self.enable_fail_fast();
        }

        trace!("start was called");
        // Everything has been scheduled and running so change the state

        let mut state = self.runtime_state.write();
        //actors all started upon spawn or team spawn when graph building
        //here we wait to ensure each one of them have a thread and are running
        state.wait_for_registrations(duration);
        if !state.is_in_state(&[GraphLivelinessState::Running]) {
            error!("timeout on startup, graph is not in the running state");
            false
        } else {
            true
        }
    }

    /// Stops the graph procedure requested.
    pub fn request_stop(&mut self) {
        let mut a = self.runtime_state.write();
        a.request_shutdown();
    }

    /// Blocks until the graph is stopped.
    ///
    /// # Arguments
    ///
    /// * `clean_shutdown_timeout` - The timeout duration for a clean shutdown.
    pub fn block_until_stopped(self, clean_shutdown_timeout: Duration) -> bool {


        if let Some(wait_on) = {
            let state = self.runtime_state.write();
            if state.is_in_state(&[GraphLivelinessState::Running, GraphLivelinessState::Building]) {
                let (tx, rx) = oneshot::channel();
                let v = state.shutdown_one_shot_vec.clone();
                abstract_executor::block_on(async move {
                    v.lock().await.push(tx);
                });
                Some(rx)
            } else {
                None
            }
        } {
            // If we are Running or Building then we block here until shutdown is started.
            let _ = abstract_executor::block_on(wait_on);
        }

        let now = Instant::now();
        // Duration is not allowed to be less than 3 frames of telemetry
        // This ensures with safety that all actors had an opportunity to
        // raise objections and delay the stop. We just take the max of both
        // durations and do not error or panic since we are shutting down
        let timeout = clean_shutdown_timeout.max(Duration::from_millis(3 * self.telemetry_production_rate_ms));

        // Wait for either the timeout or the state to be Stopped
        // While try lock then yield and do until time has passed
        // If we are stopped we will return immediately
        loop {
            let is_stopped = {
                let state = self.runtime_state.read();
                state.check_is_stopped(now, timeout)
            };
            if let Some(shutdown) = is_stopped {
                let mut state = self.runtime_state.write();
                state.state = shutdown;

                if state.state.eq(&GraphLivelinessState::StoppedUncleanly) {
                    warn!("graph stopped uncleanly");
                    Self::report_votes(&mut state);
                    return false;
                }
                return true;
            } else {
                thread::sleep(Duration::from_millis(self.telemetry_production_rate_ms));
            }
        }
    }

    fn report_votes(state: &mut RwLockWriteGuard<GraphLiveliness>) {
        warn!("voter log: (approved votes at the top, total:{})", state.votes.len());
        let mut voters = state.votes.iter()
            .map(|f| f.try_lock().map(|v| v.clone()))
            .collect::<Vec<_>>();

        // You can sort or prioritize the votes as needed here
        voters.sort_by_key(|voter| !voter.as_ref().map_or(false, |f| f.in_favor)); // This will put `true` (in favor) votes first

        // Now iterate over the sorted voters and log the results
        voters.iter().for_each(|voter| {
            warn!("#{:?} Voted: {:?} Ident: {:?}"
                                       , voter.as_ref().map_or(usize::MAX, |f| f.id)
                                       , voter.as_ref().map_or(false, |f| f.in_favor)
                                       , voter.as_ref().map_or(
                                                   Default::default()
                                                 //self.state.registered_voters[0]
                                                   , |f| f.ident));
        });
        warn!("graph stopped uncleanly");
    }

    /// Creates a new graph for normal or unit test use.
    ///
    /// # Returns
    ///
    /// A new `Graph` instance.
    pub fn internal_new<A: Any + Send + Sync>(args: A
                                              , block_fail_fast: bool
                                              , telemetry_metric_features: bool
                                              , backplane: Option<SideChannelHub>
                                              , proactor_config: Option<ProactorConfig>
                                              , enable_io_driver: bool
                                              , iouring_queue_length: u32
                                              , telemetry_production_rate_ms: u64
                                            ) -> Graph {

        let proactor_config =  if let Some(config) = proactor_config {
            config
        } else {
            ProactorConfig::InterruptDriven
        };


        //setup our threading and IO driver
        let nuclei_config = match proactor_config {
            ProactorConfig::InterruptDriven => IoUringConfiguration::interrupt_driven(iouring_queue_length),
            ProactorConfig::KernelPollDriven => IoUringConfiguration::kernel_poll_only(iouring_queue_length),
            ProactorConfig::LowLatencyDriven => IoUringConfiguration::low_latency_driven(iouring_queue_length),
            ProactorConfig::IoPoll => IoUringConfiguration::io_poll(iouring_queue_length),
        };
        abstract_executor::init(enable_io_driver, nuclei_config);

        let channel_count = Arc::new(AtomicUsize::new(0));
        let actor_count = Arc::new(AtomicUsize::new(0));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));


        let mut result = Graph {
            args: Arc::new(Box::new(args)),
            channel_count: channel_count.clone(),
            actor_count: actor_count.clone(), // This is the count of all monitors
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())), // This is all telemetry receivers
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
                                        oneshot_shutdown_vec.clone()
                                        , actor_count.clone()) ) ),
            thread_lock: Arc::new(Mutex::new(())),
            oneshot_shutdown_vec,
            backplane: Arc::new(Mutex::new(backplane)),
            noise_threshold: Instant::now().sub(Duration::from_secs(steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64)),
            block_fail_fast,
            telemetry_production_rate_ms: if telemetry_metric_features {
                                                 telemetry_production_rate_ms
                                             } else {
                                                 0u64 //this zero prevents us from building telemetry
                                             },
        };

        if telemetry_metric_features {
            telemetry::setup::build_telemetry_metric_features(&mut result);
        }
        result
    }

    /// Returns a `ChannelBuilder` for constructing new channels.
    pub fn channel_builder(&mut self) -> ChannelBuilder {
        ChannelBuilder::new(
            self.channel_count.clone(),
            self.oneshot_shutdown_vec.clone(),
            self.noise_threshold,
            self.telemetry_production_rate_ms,
        )
    }
}




#[cfg(test)]
mod graph_liveliness_tests {
    use crate::{Graph, GraphLivelinessState};

    #[test]
    fn test_fail_fast_stack_trace() {
        let mut buffer = Vec::new();
        let panic_info = "Test Panic Info";

        // Call the function with the buffer
        Graph::fail_fast_stack_trace(&panic_info, &mut buffer);

        // Convert the buffer to a string to check the output
        let output = String::from_utf8(buffer).unwrap();
        assert!(output.contains("Test Panic Info"));
    }


    #[test]
    fn test_graph_liveliness_state_equality() {
        let building = GraphLivelinessState::Building;
        let running = GraphLivelinessState::Running;
        let stop_requested = GraphLivelinessState::StopRequested;
        let stopped = GraphLivelinessState::Stopped;
        let stopped_uncleanly = GraphLivelinessState::StoppedUncleanly;

        // Ensure each state can be compared for equality and inequality
        assert_eq!(building, GraphLivelinessState::Building);
        assert_ne!(building, running);

        assert_eq!(running, GraphLivelinessState::Running);
        assert_ne!(running, stop_requested);

        assert_eq!(stop_requested, GraphLivelinessState::StopRequested);
        assert_ne!(stop_requested, stopped);

        assert_eq!(stopped, GraphLivelinessState::Stopped);
        assert_ne!(stopped, stopped_uncleanly);

        assert_eq!(stopped_uncleanly, GraphLivelinessState::StoppedUncleanly);
        assert_ne!(stopped_uncleanly, building);
    }

    #[test]
    fn test_graph_liveliness_state_cloning() {
        let building = GraphLivelinessState::Building;
        let building_clone = building.clone();

        // Verify cloning works and the clone is equal to the original
        assert_eq!(building, building_clone);
    }

    #[test]
    fn test_graph_liveliness_state_debug_output() {
        let building = GraphLivelinessState::Building;

        // Ensure Debug output works as expected
        let debug_str = format!("{:?}", building);
        assert_eq!(debug_str, "Building");
    }


}
