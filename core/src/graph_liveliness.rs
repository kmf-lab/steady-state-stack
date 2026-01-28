//! This module provides core functionalities for the SteadyState project, including the
//! graph and graph liveliness components. THE graph manages the execution of actors,
//! and the liveliness state handles the shutdown process and state transitions.

use crate::{logging_util, Troupe};
use std::ops::{Deref};
use std::sync::{Arc, OnceLock};
use parking_lot::{RwLock, RwLockWriteGuard};
use std::time::{Duration, Instant};
use futures::lock::Mutex;
use crate::core_exec;

#[allow(unused_imports)]
use log::*;
use std::any::Any;
use std::backtrace::{Backtrace};
use std::error::Error;
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use futures::channel::oneshot;
use futures::channel::oneshot::Sender;

use futures_util::lock::{MutexGuard};
use aeron::aeron::Aeron;
use aeron::context::Context;
use async_lock::Barrier;
use crate::actor_builder::{ActorBuilder, TroupeGuard};
use crate::telemetry;
use crate::channel_builder::ChannelBuilder;
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::distributed::aeron_channel_structs::aeron_utils::*;
use crate::graph_testing::StageManager;
use crate::expression_steady_eye::{i_take_expression, Eye};
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry::{metrics_collector, metrics_server};
use crate::logging_util::steady_logger;
use futures_util::FutureExt;

/// Represents the possible states of the graph's liveliness within the SteadyState framework.
///
/// This enum tracks the lifecycle of a graph, from its construction through to its shutdown,
/// reflecting the operational status of its actors.
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum GraphLivelinessState {
    /// Indicates that the graph is in the process of being constructed.
    ///
    /// During this phase, actors are being added and initialized, and the graph is not yet operational.
    Building,

    /// Indicates that the graph is fully operational and running.
    ///
    /// All actors are actively executing their designated tasks concurrently.
    Running,

    /// Indicates that a shutdown has been requested and actors are voting on it.
    ///
    /// THE graph is transitioning to a stopped state, awaiting consensus from all actors.
    StopRequested,

    /// Indicates that the graph has completely stopped cleanly.
    ///
    /// All actors have ceased execution in an orderly manner.
    Stopped,

    /// Indicates that the graph has stopped, but not all actors shut down cleanly.
    ///
    /// Some actors encountered issues during the shutdown process, leading to an incomplete stop.
    StoppedUncleanly,
}

/// Represents a vote cast by an actor regarding the shutdown of the graph.
///
/// This struct encapsulates the details of an actor's decision during the shutdown voting process,
/// including their identity and reasoning if they oppose the shutdown.
#[derive(Default)]
pub struct ShutdownVote {
    /// THE unique identifier of the actor casting the vote.
    pub(crate) id: usize,
    /// THE optional identity of the actor, providing additional context.
    pub(crate) signature: Option<ActorIdentity>,
    /// Indicates whether the actor supports the shutdown.
    pub(crate) in_favor: bool,
    /// THE current status of the voter, such as registered or dead.
    pub(crate) voter_status: VoterStatus,
    /// An optional backtrace captured if the actor vetoes the shutdown, useful for debugging.
    pub(crate) veto_backtrace: Option<Backtrace>,
    /// An optional reason provided by the actor for vetoing the shutdown.
    pub(crate) veto_reason: Option<Eye>,
}

/// Indicates the status of an actor in the voting process.
///
/// This enum defines whether an actor is actively registered, marked as dead, or not yet registered,
/// affecting its participation in shutdown votes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum VoterStatus {
    /// THE actor has not yet registered as a voter.
    #[default]
    None,
    /// THE actor is registered and eligible to vote.
    Registered(ActorIdentity),
    /// THE actor is marked as dead and cannot participate in voting.
    Dead(ActorIdentity),
}

/// Manages the liveliness state of the graph and coordinates the shutdown voting process.
///
/// This struct oversees the graph's operational state, tracks registered voters, and handles the
/// collection and evaluation of shutdown votes from actors.
pub struct GraphLiveliness {
    /// A list of statuses for all registered voters.
    pub(crate) registered_voters: Vec<VoterStatus>,
    /// THE current state of the graph's liveliness.
    pub(crate) state: GraphLivelinessState,
    /// A thread-safe collection of shutdown votes from actors.
    pub(crate) votes: Arc<Box<[Mutex<ShutdownVote>]>>,
    /// THE total number of votes in favor of shutdown.
    pub(crate) vote_in_favor_total: AtomicUsize,
    /// A shared vector of oneshot channels for sending shutdown notifications.
    pub(crate) shutdown_one_shot_vec: Arc<Mutex<Vec<Sender<()>>>>,
    /// THE count of actors currently registered as voters.
    pub(crate) registered_voter_count: AtomicUsize,
    /// A shared count of the total number of actors in the graph.
    pub(crate) actors_count: Arc<AtomicUsize>,
    /// An optional timeout duration for the shutdown process.
    pub(crate) shutdown_timeout: Option<Duration>,
}

impl GraphLiveliness {
    /// Creates a new instance of `GraphLiveliness` with an initial building state.
    ///
    /// This method sets up the necessary structures for tracking the graph's state and voter information.
    ///
    /// # Arguments
    ///
    /// * `one_shot_shutdown` - A shared vector of oneshot senders for shutdown signals.
    /// * `actors_count` - A shared counter representing the total number of actors.
    ///
    /// # Returns
    ///
    /// A newly initialized `GraphLiveliness` instance.
    pub(crate) fn new(one_shot_shutdown: Arc<Mutex<Vec<Sender<()>>>>, actors_count: Arc<AtomicUsize>) -> Self {
        GraphLiveliness {
            actors_count,
            registered_voter_count: AtomicUsize::new(0),
            registered_voters: Vec::new(),
            state: GraphLivelinessState::Building,
            votes: Arc::new(Box::new([])),
            vote_in_favor_total: AtomicUsize::new(0),
            shutdown_one_shot_vec: one_shot_shutdown,
            shutdown_timeout: None,
        }
    }

    /// Transitions the graph from the building state to the running state.
    ///
    /// This method updates the state to indicate that the graph is now operational.
    ///
    /// # Panics
    ///
    /// Panics if the current state is not `Building`, ensuring a valid state transition.
    pub(crate) fn building_to_running(&mut self) {
        if self.state.eq(&GraphLivelinessState::Building) {
            self.state = GraphLivelinessState::Running;
        } else {
            error!("unexpected state {:?}", self.state);
        }
    }

    /// Marks an actor as dead after it has exited normally.
    ///
    /// This method updates the voter status to exclude the actor from future shutdown votes.
    ///
    /// # Arguments
    ///
    /// * `ident` - THE identity of the actor to be removed from voting.
    pub(crate) fn remove_voter(&mut self, ident: ActorIdentity) {
        if self.registered_voters[ident.id].eq(&VoterStatus::Registered(ident)) {
            self.registered_voters[ident.id] = VoterStatus::Dead(ident);
        }
    }

    /// Registers an actor as a voter in the shutdown process.
    ///
    /// This method adds the actor to the list of voters, enabling it to participate in shutdown decisions.
    ///
    /// # Arguments
    ///
    /// * `ident` - THE identity of the actor to register.
    pub(crate) fn register_voter(&mut self, ident: ActorIdentity) {
        if ident.id >= self.registered_voters.len() {
            self.registered_voters.resize(ident.id + 1, VoterStatus::None);
        }
        if self.registered_voters[ident.id].eq(&VoterStatus::None) {
            self.registered_voter_count.fetch_add(1, Ordering::SeqCst);
        }
        self.registered_voters[ident.id] = VoterStatus::Registered(ident);
    }

    /// Waits for all actors to register as voters within a specified timeout.
    ///
    /// This method blocks until all actors have registered or the timeout is exceeded, then transitions to running.
    ///
    /// # Arguments
    ///
    /// * `timeout` - THE maximum duration to wait for actor registration.
    pub(crate) fn wait_for_registrations(&mut self, timeout: Duration) {
        if self.actors_count.load(Ordering::SeqCst) > 0 {
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
            #[cfg(not(test))]
            warn!("This graph contains no actors.");
        }
        trace!("changed to running state");
        self.building_to_running();
    }

    /// Initiates a shutdown request for the graph and notifies all actors.
    ///
    /// This method transitions the graph to the `StopRequested` state and triggers voting among actors.
    ///
    /// # Arguments
    ///
    /// * `runtime_state` - A shared reference to the graph's liveliness state.
    pub(crate) async fn internal_request_shutdown(runtime_state: Arc<RwLock<GraphLiveliness>>) {
        error!("starting shutdown reqeuests");
        if runtime_state.read().state.eq(&GraphLivelinessState::Running) {
            let read = runtime_state.read();
            let votes: Vec<Mutex<ShutdownVote>> = read.registered_voters.iter().enumerate().map(|(i, v)| {
                Mutex::new(ShutdownVote {
                    id: i,
                    signature: None,
                    in_favor: false,
                    voter_status: v.clone(),
                    veto_backtrace: None,
                    veto_reason: None,
                })
            }).collect();
            let local_oss = read.shutdown_one_shot_vec.clone();
            drop(read);
            let mut write = runtime_state.write();
            write.votes = Arc::new(votes.into_boxed_slice());
            write.vote_in_favor_total.store(0, Ordering::SeqCst);
            write.state = GraphLivelinessState::StopRequested;
            drop(write);
            GraphLiveliness::vote_for_the_dead(runtime_state);
            let mut one_shots: MutexGuard<Vec<Sender<_>>> = local_oss.lock().await;
            while let Some(f) = one_shots.pop() {
                let _ignore = f.send(());
                error!("send one shot {:?}",_ignore);
            }
            error!("finished all shutdown reqeuests");
            trace!("every actor has had one shot shutdown fired now");
        } else if runtime_state.read().is_in_state(&[GraphLivelinessState::Building]) {
            warn!("request_shutdown should only be called after start");
        }
    }

    /// Automatically casts votes in favor of shutdown for actors marked as dead.
    ///
    /// This method ensures that inactive actors do not impede the shutdown process.
    ///
    /// # Arguments
    ///
    /// * `runtime_state` - A shared reference to the graph's liveliness state.
    pub(crate) fn vote_for_the_dead(runtime_state: Arc<RwLock<GraphLiveliness>>) {
        let read = runtime_state.read();
        let the_dead: Vec<(usize, ActorIdentity)> = read.registered_voters.iter().enumerate().flat_map(|(i, v)| {
            if let VoterStatus::Dead(ident) = v {
                let my_ballot = &read.votes[i];
                if let Some(vote) = my_ballot.try_lock() {
                    if !vote.in_favor {
                        Some((i, *ident))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        }).collect();
        drop(read);
        if !the_dead.is_empty() {
            let write = runtime_state.write();
            the_dead.iter().for_each(|(i, ident)| {
                let my_ballot = &write.votes[*i];
                if let Some(mut vote) = my_ballot.try_lock() {
                    assert_eq!(vote.id, *i);
                    vote.signature = Some(*ident);
                    vote.in_favor = true;
                    write.vote_in_favor_total.fetch_add(1, Ordering::SeqCst);
                } else {
                    error!("voting integrity error, someone else has my ballot {:?} in_favor of shutdown", ident);
                }
            })
        }
    }

    /// Checks whether the graph has reached a stopped state based on votes and timeout.
    ///
    /// This method evaluates if the shutdown process has completed successfully or timed out.
    ///
    /// # Arguments
    ///
    /// * `now` - THE instant when the shutdown was initiated.
    /// * `timeout` - THE maximum duration allowed for a clean shutdown.
    ///
    /// # Returns
    ///
    /// An optional new state if the graph has stopped, or `None` if still in progress.
    pub fn check_is_stopped(&self, now: Instant, timeout: Duration) -> Option<GraphLivelinessState> {
        if self.is_in_state(&[
            GraphLivelinessState::StopRequested,
            GraphLivelinessState::Stopped,
            GraphLivelinessState::StoppedUncleanly,
        ]) {
            let voters_count = self.votes.len();
            if self.vote_in_favor_total.load(Ordering::SeqCst) == voters_count {
                Some(GraphLivelinessState::Stopped)
            } else if (voters_count>0) && (now.elapsed()>timeout) {
                Some(GraphLivelinessState::StoppedUncleanly)
            } else {
                if voters_count>0 {
                    None
                } else {
                    assert_eq!(0,voters_count);
                    Some(GraphLivelinessState::Stopped)
                }
            }
        } else {
            None
        }
    }

    /// Determines if the graph is currently in one of the specified states.
    ///
    /// This method checks the current state against a list of possible states.
    ///
    /// # Arguments
    ///
    /// * `matches` - A slice of states to check against.
    ///
    /// # Returns
    ///
    /// `true` if the current state matches any of the provided states, `false` otherwise.
    pub fn is_in_state(&self, matches: &[GraphLivelinessState]) -> bool {
        matches.iter().any(|f| f.eq(&self.state))
    }

    /// Checks if all actors except the telemetry system (Collector and Server) have voted 'yes'.
    /// This is used by telemetry actors to ensure they are the last to shut down,
    /// allowing them to capture the final state of all other actors.
    ///
    /// # Panics
    /// Panics if there are fewer than 2 actors, as the telemetry system itself requires
    /// both a Collector and a Server.
    pub fn is_shutdown_telemetry_complete(&self, count: usize) -> bool {
        let total = self.votes.len();
        assert!(total >= count, "Invariant failure: Telemetry system requires at least {:?} actors (ie, Collector and Server)", count);
        let yes_votes = self.vote_in_favor_total.load(Ordering::Relaxed);
        yes_votes >= total - count
    }

    /// Assesses whether an actor should continue running based on the graph's state and its vote.
    ///
    /// This method helps actors determine their operational status during state transitions.
    ///
    /// # Arguments
    ///
    /// * `ident` - THE identity of the actor querying its status.
    /// * `accept_fn` - A closure that determines if the actor accepts the shutdown request.
    ///
    /// # Returns
    ///
    /// `Some(true)` if the actor should keep running, `Some(false)` if it should stop, or `None` if still building.
    pub(crate) fn is_running<F: FnMut() -> bool>(&self, ident: ActorIdentity, mut accept_fn: F) -> Option<bool> {
        match self.state {
            GraphLivelinessState::Building => {
                thread::yield_now();
                None
            }
            GraphLivelinessState::Running => Some(true),
            GraphLivelinessState::StopRequested => {
                let my_ballot = &self.votes[ident.id];
                if let Some(mut vote) = my_ballot.try_lock() {


                    debug_assert_eq!(vote.id, ident.id);
                    let in_favor = accept_fn(); //has side effect, must act on results!
                    if in_favor {
                            error!("now agreed to shutdown: {:?}",&ident);
                    }
                    vote.signature = Some(ident);
                    if in_favor && !vote.in_favor {
                        self.vote_in_favor_total.fetch_add(1, Ordering::SeqCst);
                        vote.veto_backtrace = None;
                        vote.in_favor = in_favor;
                    } else {
                        //if cfg!(debug_assertions) { //TODO: noise!, not the best feature
                        //    vote.veto_backtrace = Some(Backtrace::capture());
                        //}
                        vote.veto_reason = i_take_expression();
                        if vote.in_favor {
                            warn!("already voted in favor! : {:?} {:?} vs {:?}", ident, in_favor, vote.in_favor);
                        }
                    }
                    drop(vote);
                    Some(!in_favor)
                } else {
                 //   error!("2 hello {:?}",&ident);

                    trace!("just try again later, unable to get the lock");
                    Some(true)
                }
            }
            GraphLivelinessState::Stopped | GraphLivelinessState::StoppedUncleanly => Some(false),
        }
    }
}

/// Identifies an actor within the graph uniquely.
///
/// This struct combines a numeric identifier with a human-readable name for actor distinction.
#[derive(Clone, Default, Copy, PartialEq, Eq, Hash)]
pub struct ActorIdentity {
    /// A unique numeric identifier for the actor within the graph.
    pub id: usize,
    /// THE human-readable name of the actor, potentially with a suffix for uniqueness.
    pub label: ActorName,
}

/// Represents the name of an actor, including an optional suffix for uniqueness.
///
/// This struct provides a static base name and an optional numeric suffix to differentiate actors.
#[derive(Clone, Default, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ActorName {
    /// THE static, immutable base name of the actor.
    pub name: &'static str,
    /// An optional numeric suffix to ensure uniqueness among actors with the same base name.
    pub suffix: Option<usize>,
}

impl ActorIdentity {
    /// Constructs a new `ActorIdentity` with the specified parameters.
    ///
    /// This method creates an identity for an actor using a unique ID and a name with an optional suffix.
    ///
    /// # Arguments
    ///
    /// * `id` - THE unique numeric identifier for the actor.
    /// * `name` - THE static base name of the actor.
    /// * `suffix` - An optional numeric suffix for uniqueness.
    ///
    /// # Returns
    ///
    /// A new `ActorIdentity` instance.
    pub fn new(id: usize, name: &'static str, suffix: Option<usize>) -> Self {
        ActorIdentity {
            id,
            label: ActorName { name, suffix },
        }
    }
}

impl ActorName {
    /// Constructs a new `ActorName` with the specified name and optional suffix.
    ///
    /// This method creates a name structure for an actor, allowing for differentiation via a suffix.
    ///
    /// # Arguments
    ///
    /// * `name` - THE static base name of the actor.
    /// * `suffix` - An optional numeric suffix for uniqueness.
    ///
    /// # Returns
    ///
    /// A new `ActorName` instance.
    pub fn new(name: &'static str, suffix: Option<usize>) -> Self {
        ActorName { name, suffix }
    }
}

impl Debug for ActorIdentity {
    /// Formats the `ActorIdentity` for debugging purposes.
    ///
    /// This implementation provides a string representation including the ID and name, with suffix if present.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:?} {}", self.id, self.label.name)?;
        if let Some(suffix) = self.label.suffix {
            write!(f, "-{}", suffix)?;
        }
        Ok(())
    }
}

/// Defines configuration options for the proactor, which manages I/O operations.
///
/// This enum specifies different strategies for handling I/O, each tailored to specific performance needs.
#[derive(Clone, Debug)]
pub enum ProactorConfig {
    /// Configures the proactor for interrupt-driven I/O with minimal CPU usage.
    ///
    /// This option is suitable for low-throughput scenarios where completion latency is less critical.
    InterruptDriven,
    /// Configures the proactor to use kernel polling for efficient I/O in high-traffic environments.
    ///
    /// This balances throughput and resource usage without aggressive polling.
    KernelPollDriven,
    /// Configures the proactor for low-latency, high-throughput I/O operations.
    ///
    /// This option prioritizes performance, consuming more resources for demanding workloads.
    LowLatencyDriven,
    /// Configures the proactor with I/O polling for low-latency file operations.
    ///
    /// This is optimized for file-based I/O, reducing latency in such contexts.
    IoPoll,
}

/// Configures and builds a `Graph` instance with customizable options.
///
/// This struct allows setting up the graph for either production or testing environments, adjusting
/// parameters like telemetry and I/O behavior.
#[derive(Clone, Debug)]
pub struct GraphBuilder {
    /// Indicates whether the graph is intended for testing purposes.
    is_for_testing: bool,
    /// Enables or disables telemetry metric features.
    telemetry_metric_features: bool,
    /// Enables or disables the I/O driver.
    enable_io_driver: bool,
    /// An optional backplane for testing side-channel communications.
    backplane: Option<StageManager>,
    /// THE configuration for the proactor, if specified.
    proactor_config: Option<ProactorConfig>,
    /// THE queue length for I/O uring operations.
    iouring_queue_length: u32,
    /// THE rate at which telemetry data is produced, in milliseconds.
    telemtry_production_rate_ms: u64,
    /// An optional hex color for the telemetry top bar.
    telemetry_colors: Option<(String, String)>,
    /// An optional barrier for synchronizing actor shutdown.
    shutdown_barrier: Option<Arc<Barrier>>,
    /// Default stack size for all actors in the graph.
    default_stack_size: Option<usize>,
    /// Flag to block fail-fast behavior during tests.
    block_fail_fast: bool,
    /// Minimum size for bundles.
    bundle_floor_size: usize,
}

impl Default for GraphBuilder {
    /// Provides a default `GraphBuilder` configured for production use.
    ///
    /// This implementation returns a builder with production-ready settings.
    fn default() -> Self {
        GraphBuilder::for_production()
    }
}

impl GraphBuilder {
    /// Creates a `GraphBuilder` configured for production environments.
    ///
    /// This method sets up a builder with defaults optimized for production use, such as enabling the I/O driver.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance tailored for production.
    pub fn for_production() -> Self {
        #[cfg(test)]
        panic!("should not call for_production in tests");
        #[cfg(not(test))]
        GraphBuilder {
            is_for_testing: false,
            telemetry_metric_features: crate::steady_config::TELEMETRY_SERVER,
            enable_io_driver: true,
            backplane: None,
            proactor_config: Some(ProactorConfig::InterruptDriven),
            iouring_queue_length: 1 << 5,
            telemtry_production_rate_ms: 40,
            telemetry_colors: None,
            shutdown_barrier: None,
            default_stack_size: None,
            block_fail_fast: false,
            bundle_floor_size: 4,
        }
    }

    /// Creates a `GraphBuilder` configured for testing environments.
    ///
    /// This method sets up a builder with defaults suitable for testing, including a backplane for side channels.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance tailored for testing.
    pub fn for_testing() -> Self {
        let _ = logging_util::steady_logger::initialize();
        GraphBuilder {
            is_for_testing: true,
            telemetry_metric_features: false,
            enable_io_driver: false,
            backplane: Some(StageManager::default()),
            proactor_config: Some(ProactorConfig::InterruptDriven),
            iouring_queue_length: 1 << 5,
            telemtry_production_rate_ms: 40,
            telemetry_colors: None,
            shutdown_barrier: None,
            default_stack_size: None,
            block_fail_fast: true,
            bundle_floor_size: 4,
        }
    }

    /// Sets the queue length for I/O uring operations.
    ///
    /// This method adjusts the capacity for I/O operations, which may need to be increased for high workloads.
    ///
    /// # Arguments
    ///
    /// * `len` - THE desired queue length.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance with the updated queue length.
    pub fn with_iouring_queue_length(&self, len: u32) -> Self {
        let mut result = self.clone();
        result.iouring_queue_length = len;
        result
    }

    /// Sets the telemetry production rate in milliseconds.
    ///
    /// This method configures how frequently telemetry data is generated, with a minimum of 40ms.
    ///
    /// # Arguments
    ///
    /// * `ms` - THE desired production rate in milliseconds.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance with the updated telemetry rate.
    pub fn with_telemtry_production_rate_ms(&self, ms: u64) -> Self {
        let mut result = self.clone();
        if ms >= 40 {
            result.telemtry_production_rate_ms = ms;
        } else {
            warn!("telemetry production rate must be at least 40ms, setting to 40ms");
        }
        result
    }

    /// Sets the telemetry top bar colors (primary and secondary hex strings).
    pub fn with_telemetry_colors(&self, primary: &str, secondary: &str) -> Self {
        let mut result = self.clone();
        result.telemetry_colors = Some((primary.to_string(), secondary.to_string()));
        result
    }

    /// Configures a shutdown barrier to synchronize actor shutdown.
    ///
    /// This method sets up a barrier to ensure all actors reach a shutdown point together.
    ///
    /// # Arguments
    ///
    /// * `latched_actor_count` - THE number of actors to synchronize.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance with the shutdown barrier configured.
    pub fn with_shutdown_barrier(&self, latched_actor_count: usize) -> Self {
        let mut result = self.clone();
        result.shutdown_barrier = Some(Arc::new(Barrier::new(latched_actor_count)));
        result
    }

    /// Sets the default stack size for all actors in the graph.
    ///
    /// # Arguments
    ///
    /// * `mb` - THE desired stack size in megabytes.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance with the updated stack size.
    pub fn with_default_actor_stack_size(&self, bytes_count: usize) -> Self {
        let mut result = self.clone();
        result.default_stack_size = Some(bytes_count);
        result
    }

    /// Disables the fail-fast behavior (process exit on panic) for the graph.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance with fail-fast behavior blocked.
    pub fn with_block_fail_fast(&self) -> Self {
        let mut result = self.clone();
        result.block_fail_fast = true;
        result
    }

    /// Sets the threshold for bundling edges in the telemetry visualization.
    pub fn with_aggregation_threshold(&self, threshold: usize) -> Self {
        let mut result = self.clone();
        result.bundle_floor_size = threshold;
        result
    }

    /// Sets the minimum size for bundles.
    pub fn with_bundle_floor_size(&self, size: usize) -> Self {
        let mut result = self.clone();
        result.bundle_floor_size = size;
        result
    }

    /// Enables or disables telemetry metric features.
    ///
    /// This method toggles telemetry support, enabling the I/O driver if telemetry is activated.
    ///
    /// # Arguments
    ///
    /// * `enable` - Whether to enable telemetry metric features.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance with updated telemetry settings.
    pub fn with_telemetry_metric_features(&self, enable: bool) -> Self {
        let mut result = self.clone();
        result.telemetry_metric_features = enable;
        if enable {
            result.enable_io_driver = true;
        }
        result
    }

    /// Builds a `Graph` instance based on the configured settings.
    ///
    /// This method consumes the builder and constructs a graph with the provided arguments.
    ///
    /// # Type Parameters
    ///
    /// * `A` - THE type of arguments, which must implement `Any`, `Send`, and `Sync`.
    ///
    /// # Arguments
    ///
    /// * `args` - THE arguments to pass to the graph during construction.
    ///
    /// # Returns
    ///
    /// A fully configured `Graph` instance.
    pub fn build<A: Any + Send + Sync>(self, args: A) -> Graph {
        let g = Graph::internal_new(args, self.clone());
        #[cfg(feature = "disable_actor_restart_on_failure")]
        {
            if !self.block_fail_fast {
                g.apply_fail_fast();
                trace!("fail fast enabled for testing !");
            }
        }

        let ctrlc_runtime_state = g.runtime_state.clone();
        let tel_prod_rate = Duration::from_millis(g.telemetry_production_rate_ms);
        let result = ctrlc::set_handler(move || {
            println!("Ctrl-C received, initiating shutdown...");
            let now = Instant::now();
            let timeout = {
                let value1 = ctrlc_runtime_state.clone();
                let value2 = ctrlc_runtime_state.clone();
                core_exec::block_on(async move { GraphLiveliness::internal_request_shutdown(value1).await });
                if let Some(timeout) = value2.read().shutdown_timeout {
                    timeout
                } else {
                    Duration::from_secs(1)
                }
            };
            let _ = Graph::watch_shutdown(timeout, now, ctrlc_runtime_state.clone(), tel_prod_rate);
        });
        if let Err(e) = result {
            trace!("Error setting up CTRL-C hook: {}", e);
        }
        g
    }
}

/// Represents the graph of actors and manages their execution and lifecycle.
///
/// This struct orchestrates the actors within the SteadyState framework, handling their startup,
/// execution, telemetry, and shutdown processes.
pub struct Graph {
    /// THE arguments passed to the graph, stored in a thread-safe manner.
    pub(crate) args: Arc<Box<dyn Any + Send + Sync>>,
    /// A shared counter for the number of channels in the graph.
    pub(crate) channel_count: Arc<AtomicUsize>,
    /// A shared counter for the number of actors in the graph.
    pub(crate) actor_count: Arc<AtomicUsize>,
    /// A mutex for synchronizing thread operations.
    pub(crate) thread_lock: Arc<Mutex<()>>,
    /// A shared counter for the number of actor troupes.
    pub(crate) team_count: Arc<AtomicUsize>,
    /// Indicates whether the graph is configured for testing.
    pub(crate) is_for_testing: bool,
    /// A collection of telemetry receivers for monitoring the graph.
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    /// THE shared liveliness state of the graph.
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    /// A shared vector of oneshot senders for shutdown notifications.
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    /// An optional backplane for testing side-channel communications.
    pub(crate) backplane: Arc<Mutex<Option<StageManager>>>,
    /// THE rate at which telemetry data is produced, in milliseconds.
    pub(crate) telemetry_production_rate_ms: u64,
    /// An optional hex color for the telemetry top bar.
    pub(crate) telemetry_colors: Option<(String, String)>,
    /// A lazily initialized reference to the Aeron media driver.
    pub(crate) aeron: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    /// An optional barrier for synchronizing actor shutdown.
    pub shutdown_barrier: Option<Arc<Barrier>>,
    /// Default stack size for all actors in the graph.
    pub(crate) default_stack_size: Option<usize>,
    /// Minimum size for bundles.
    pub(crate) bundle_floor_size: usize,
}

/// A guard that provides access to the stage manager for testing purposes.
///
/// This struct holds a lock on the backplane, allowing test code to interact with it safely.
pub struct StageManagerGuard<'a> {
    /// THE mutex guard holding the lock on the backplane.
    guard: MutexGuard<'a, Option<StageManager>>,
}

impl Deref for StageManagerGuard<'_> {
    type Target = StageManager;

    /// Provides immutable access to the underlying stage manager.
    ///
    /// This allows dereferencing the guard to interact with the stage manager directly.
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().expect("SideChannelHub is not initialized")
    }
}

impl StageManagerGuard<'_> {
    /// Releases the lock on the stage manager explicitly.
    ///
    /// This method allows the guard to be dropped manually, freeing the lock for other operations.
    pub fn final_bow(self) {
    }
}

impl Graph {
    /// Acquires a lock on the stage manager for testing purposes.
    ///
    /// This method provides a guard that allows interaction with the backplane in a test environment.
    ///
    /// # Returns
    ///
    /// A `StageManagerGuard` that holds the lock until dropped.
    pub fn stage_manager(&self) -> StageManagerGuard<'_> {
        let guard = core_exec::block_on(self.backplane.lock());
        StageManagerGuard { guard }
    }

    /// Retrieves the Aeron media driver, initializing it if necessary.
    ///
    /// This method attempts to access or establish the media driver for communication purposes.
    ///
    /// # Returns
    ///
    /// An optional reference to the media driver, or `None` if unavailable.
    pub fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> {
        Self::aeron_media_driver_internal(&self.aeron)
    }

    /// Internal helper to retrieve or initialize the Aeron media driver.
    ///
    /// This method manages the lazy initialization of the media driver with retry logic.
    ///
    /// # Arguments
    ///
    /// * `holder` - THE `OnceLock` containing the media driver instance.
    ///
    /// # Returns
    ///
    /// An optional reference to the media driver.
    pub(crate) fn aeron_media_driver_internal(holder: &OnceLock<Option<Arc<Mutex<Aeron>>>>) -> Option<Arc<Mutex<Aeron>>> {
        holder.get_or_init(|| aeron_context_with_retry(Context::new(), Duration::from_secs(60), Duration::from_millis(50))).clone()
    }

    /// Sets the logging level for the graph's operations.
    ///
    /// This method configures the verbosity of log output for the graph.
    ///
    /// # Arguments
    ///
    /// * `loglevel` - THE desired logging level to apply.
    pub fn loglevel(&self, loglevel: crate::LogLevel) {
        let _ = steady_logger::initialize_with_level(loglevel);
    }

    /// Attempts to retrieve the graph's arguments cast to a specific type.
    ///
    /// This method allows accessing the arguments provided during graph construction.
    ///
    /// # Type Parameters
    ///
    /// * `A` - THE type to which the arguments should be cast.
    ///
    /// # Returns
    ///
    /// An optional reference to the arguments if the cast succeeds, or `None` if it fails.
    pub fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    /// Creates a test monitor for use in testing scenarios.
    ///
    /// This method constructs a monitor that operates independently of a full graph, intended for testing only.
    ///
    /// # Arguments
    ///
    /// * `name` - THE name to assign to the test monitor.
    ///
    /// # Returns
    ///
    /// A `SteadyActorShadow` instance configured for testing.
    pub fn new_testing_test_monitor(&self, name: &'static str) -> SteadyActorShadow {
        info!("this is for testing only, never run as part of your release");
        let channel_count = self.channel_count.clone();
        let all_telemetry_rx = self.all_telemetry_rx.clone();
        let oneshot_shutdown = {
            let (send_shutdown_notice, rx) = oneshot::channel();
            let local_vec = self.oneshot_shutdown_vec.clone();
            let runtime_state = self.runtime_state.clone();
            core_exec::block_on(async move {
                let mut v = local_vec.lock().await;
                // If the graph is already in StopRequested state, fire the signal immediately
                // for this new actor instance. This ensures that actors born during the 
                // shutdown window (e.g. after a panic) don't miss the global signal.
                if runtime_state.read().is_in_state(&[GraphLivelinessState::StopRequested]) {
                    let _ = send_shutdown_notice.send(());
                } else {
                    v.push(send_shutdown_notice);
                }
            });
            rx.shared()
        };
        let now = Instant::now();
        SteadyActorShadow {
            channel_count,
            ident: ActorIdentity::new(usize::MAX, name, None),
            args: self.args.clone(),
            is_in_graph: false,
            actor_metadata: Arc::new(ActorMetaData::default()),
            all_telemetry_rx,
            runtime_state: self.runtime_state.clone(),
            regeneration: 0,
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown,
            last_periodic_wait: Default::default(),
            actor_start_time: now,
            node_tx_rx: None,
            frame_rate_ms: self.telemetry_production_rate_ms,
            team_id: 0,
            show_thread_info: false,
            aeron_meda_driver: self.aeron.clone(),
            use_internal_behavior: true,
            shutdown_barrier: self.shutdown_barrier.clone(),
        }
    }

    /// Creates a new `ActorBuilder` for constructing actors within the graph.
    ///
    /// This method provides a builder to define and initialize new actors.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance linked to this graph.
    pub fn actor_builder(&mut self) -> ActorBuilder {
        ActorBuilder::new(self)
    }

    /// Creates a `TroupeGuard` for managing a group of actors that execute together.
    ///
    /// This method sets up a troupe that will be spawned when the guard is dropped.
    ///
    /// # Returns
    ///
    /// A `TroupeGuard` instance for managing the actor troupe.
    pub fn actor_troupe(&self) -> TroupeGuard {
        TroupeGuard {
            troupe: Some(Troupe::new(self)),
        }
    }

    /// Applies fail-fast behavior by setting a panic hook that exits immediately on panic.
    ///
    /// This method is active only in debug builds and can be disabled via configuration.
    #[cfg(feature = "disable_actor_restart_on_failure")]
    fn apply_fail_fast(&self) {
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic_info| {
                default_hook(panic_info);
                std::process::exit(-1);
            }));
    }

    /// Starts the graph with a default timeout of 40 seconds for actor registration.
    ///
    /// This method initiates the graph's operation, waiting for actors to register before proceeding.
    pub fn start(&mut self) {
        self.start_with_timeout(Duration::from_secs(40));
    }

    /// Starts the graph with a specified timeout for actor registration.
    ///
    /// This method initiates the graph and waits for all actors to register within the given duration.
    ///
    /// # Arguments
    ///
    /// * `duration` - THE maximum time to wait for actor registration.
    ///
    /// # Returns
    ///
    /// `true` if all actors registered within the timeout, `false` otherwise.
    pub fn start_with_timeout(&mut self, duration: Duration) -> bool {
        trace!("start was called");
        let mut state = self.runtime_state.write();
        state.wait_for_registrations(duration);
        if !state.is_in_state(&[GraphLivelinessState::Running]) {
            error!("timeout on startup, graph is not in the running state");
            false
        } else {
            true
        }
    }

    /// Requests the shutdown of the graph, notifying all actors.
    ///
    /// This method initiates the shutdown process, triggering the voting mechanism among actors.
    pub fn request_shutdown(&mut self) {
        let a = self.runtime_state.clone();
        core_exec::block_on(async move { GraphLiveliness::internal_request_shutdown(a).await });
    }

    /// Blocks the current thread until the graph has fully stopped.
    ///
    /// This method waits for the shutdown process to complete, either cleanly or uncleanly.
    ///
    /// # Arguments
    ///
    /// * `clean_shutdown_timeout` - THE maximum duration to wait for a clean shutdown.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the shutdown was clean, or an error if it was unclean.
    pub fn block_until_stopped(self, clean_shutdown_timeout: Duration) -> Result<(), Box<dyn std::error::Error>> {
        let timeout = clean_shutdown_timeout.max(Duration::from_millis(3 * self.telemetry_production_rate_ms));
        if let Some(wait_on) = {
            self.runtime_state.write().shutdown_timeout = Some(timeout);
            if self.runtime_state.read().is_in_state(&[GraphLivelinessState::Running, GraphLivelinessState::Building]) {
                let (tx, rx) = oneshot::channel();
                let v = self.runtime_state.read().shutdown_one_shot_vec.clone();
                core_exec::block_on(async move {
                    v.lock().await.push(tx);
                });
                Some(rx)
            } else {
                None
            }
        } {
            let _ = core_exec::block_on(wait_on);
        }
        let now = Instant::now();
        let rs = self.runtime_state;
        let tel_prod_rate = Duration::from_millis(self.telemetry_production_rate_ms);
        Self::watch_shutdown(timeout, now, rs, tel_prod_rate)
    }

    /// Monitors the shutdown process until completion or timeout.
    ///
    /// This method periodically checks the graph's state, updating it based on votes and timing out if necessary.
    ///
    /// # Arguments
    ///
    /// * `timeout` - THE maximum duration for a clean shutdown.
    /// * `now` - THE time at which shutdown was initiated.
    /// * `rs` - A shared reference to the graph's liveliness state.
    /// * `tel_prod_rate` - THE interval at which to check the shutdown status.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the shutdown was clean, or an error if it was unclean.
    fn watch_shutdown(timeout: Duration, now: Instant, rs: Arc<RwLock<GraphLiveliness>>, tel_prod_rate: Duration) -> Result<(), Box<dyn Error>> {
        loop {
            let is_stopped = rs.read().check_is_stopped(now, timeout);
            if let Some(shutdown) = is_stopped {
                let is_unclean = shutdown.eq(&GraphLivelinessState::StoppedUncleanly);
                rs.write().state = shutdown;
                if is_unclean {
                    warn!("graph stopped uncleanly: votes report");
                    Self::report_votes(&mut rs.write());
                    return Err("graph stopped uncleanly error from watch_shutdown".into());
                }
                return Ok(());
            } else {
                thread::sleep(tel_prod_rate);
                GraphLiveliness::vote_for_the_dead(rs.clone());
            }
        }
    }

    /// Logs the results of the shutdown voting process for debugging.
    ///
    /// This method reports the votes and any veto details when the graph stops uncleanly.
    ///
    /// # Arguments
    ///
    /// * `state` - A mutable reference to the graph's liveliness state under a write lock.
    fn report_votes(state: &mut RwLockWriteGuard<GraphLiveliness>) {
        warn!("voter log: (approved votes at the top, total:{})", state.votes.len());
        let mut voters = state.votes.iter().map(|f| f.try_lock()).collect::<Vec<_>>();
        voters.sort_by_key(|voter| !voter.as_ref().is_some_and(|f| f.in_favor));
        voters.iter().for_each(|voter| {
            warn!("#{:?} Status:{:?} Voted: {:?} {:?} Ident: {:?}"
                   , voter.as_ref().map_or(usize::MAX, |f| f.id)
                   , voter.as_ref().map_or(Default::default(), |f| f.voter_status.clone())
                   , voter.as_ref().is_some_and(|f| f.in_favor)
                   , if voter.as_ref().is_some_and(|f| f.in_favor)
                                {"".to_string()} else
                                {voter.as_ref().map_or(None, |f| f.veto_reason.clone()).map_or("".to_string(), |f| f.veto_reason())}
                   , voter.as_ref().map_or(Default::default(), |f| f.signature));
        });
        warn!("graph stopped uncleanly, with voters {}",voters.len());
        voters.iter().for_each(|voter| {
            let signature = voter.as_ref().map_or(&None, |f| &f.signature);
            let skip_internal = if let Some(signature) = signature {
                (metrics_server::NAME == signature.label.name) || (metrics_collector::NAME == signature.label.name)
            } else {
                false
            };
            if !skip_internal {
                let backtrace = voter.as_ref().map_or(&None, |f| &f.veto_backtrace);
                let is_veto = !voter.as_ref().is_some_and(|f| f.in_favor);
                if is_veto {
                    let reason = voter.as_ref().map_or(&None, |f| &f.veto_reason);
                    if let Some(r) = reason {
                        debug!("veto expression: {:#?}", r.veto_reason());
                    }
                    if let Some(bt) = backtrace {
                        let text = format!("{:#?}", bt);
                        let adj = text.trim();
                        let adj = adj.strip_prefix("Backtrace ").unwrap_or(adj);
                        let adj = adj.strip_prefix("[").unwrap_or(adj).trim();
                        let adj = adj.strip_suffix("]").unwrap_or(adj).trim();
                        let mut level = 1;
                        let mut is_header = true;
                        let mut start = 0;
                        for (i, c) in adj.char_indices() {
                            if c == '{' {
                                level += 1;
                            } else if c == '}' {
                                level -= 1;
                            }
                            if c == ',' && level == 1 {
                                let end = i;
                                let frame = &adj[start..end];
                                let frame = frame.trim();
                                if is_header
                                    && !frame.starts_with("{ fn: \"std::backtrace")
                                    && !frame.starts_with("{ fn: \"steady_state::graph_liveliness::GraphLiveliness::is_running")
                                    && !frame.starts_with("{ fn: \"steady_state::commander_") {
                                    is_header = false;
                                }
                                if !is_header {
                                    debug!("{}", frame);
                                    if frame.starts_with("{ fn: \"steady_state::actor_builder::launch_actor") {
                                        break;
                                    }
                                }
                                start = i + 1;
                            }
                        }
                    }
                    debug!("\n\n");
                }
            }
        });
    }

    /// Constructs a new `Graph` instance based on provided arguments and builder configuration.
    ///
    /// This method initializes the graph with all necessary components for actor execution and management.
    ///
    /// # Type Parameters
    ///
    /// * `A` - THE type of arguments, which must implement `Any`, `Send`, and `Sync`.
    ///
    /// # Arguments
    ///
    /// * `args` - THE arguments to initialize the graph with.
    /// * `builder` - THE `GraphBuilder` providing configuration options.
    ///
    /// # Returns
    ///
    /// A new `Graph` instance ready for use.
    pub fn internal_new<A: Any + Send + Sync>(args: A, builder: GraphBuilder) -> Graph {
        let proactor_config = if let Some(config) = builder.proactor_config {
            config
        } else {
            ProactorConfig::InterruptDriven
        };
        core_exec::init(builder.enable_io_driver, proactor_config, builder.iouring_queue_length);
        let channel_count = Arc::new(AtomicUsize::new(0));
        let actor_count = Arc::new(AtomicUsize::new(0));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
        let mut result = Graph {
            args: Arc::new(Box::new(args)),
            channel_count: channel_count.clone(),
            actor_count: actor_count.clone(),
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())),
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
                oneshot_shutdown_vec.clone(),
                actor_count.clone(),
            ))),
            thread_lock: Arc::new(Mutex::new(())),
            oneshot_shutdown_vec,
            backplane: Arc::new(Mutex::new(builder.backplane)),
            telemetry_production_rate_ms: if builder.telemetry_metric_features {
                builder.telemtry_production_rate_ms
            } else {
                0u64
            },
            telemetry_colors: builder.telemetry_colors,
            team_count: Arc::new(AtomicUsize::new(1)),
            aeron: Default::default(),
            is_for_testing: builder.is_for_testing,
            shutdown_barrier: builder.shutdown_barrier,
            default_stack_size: builder.default_stack_size,
            bundle_floor_size: builder.bundle_floor_size,
        };
        if builder.telemetry_metric_features {
            telemetry::setup::build_telemetry_metric_features(&mut result);
        }
        result
    }

    /// Creates a new `ChannelBuilder` for constructing channels within the graph.
    ///
    /// This method provides a builder to define and initialize communication channels.
    ///
    /// # Returns
    ///
    /// A new `ChannelBuilder` instance linked to this graph.
    pub fn channel_builder(&mut self) -> ChannelBuilder {
        ChannelBuilder::new(
            self.channel_count.clone(),
            self.oneshot_shutdown_vec.clone(),
            self.telemetry_production_rate_ms,
        )
    }
}

#[cfg(test)]
mod graph_liveliness_tests {
    use crate::{GraphLivelinessState, GraphBuilder, ScheduleAs, SteadyActor};
    use std::time::Duration;

    #[test]
    fn test_unclean_shutdown_veto() {
        let mut graph = GraphBuilder::for_testing().build(());
        
        graph.actor_builder()
            .with_name("VetoActor")
            .build(|mut actor| {
                Box::pin(async move {
                    // Veto shutdown by returning false in the accept_fn
                    while actor.is_running(|| false) {
                        actor.wait(Duration::from_millis(10)).await;
                    }
                    Ok(())
                })
            }, ScheduleAs::SoloAct);

        graph.start();
        graph.request_shutdown();
        
        // This should return an error because the actor vetoed
        let result = graph.block_until_stopped(Duration::from_millis(100));
        assert!(result.is_err());
    }

    #[test]
    fn test_graph_liveliness_state_equality() {
        let building = GraphLivelinessState::Building;
        let running = GraphLivelinessState::Running;
        let stop_requested = GraphLivelinessState::StopRequested;
        let stopped = GraphLivelinessState::Stopped;
        let stopped_uncleanly = GraphLivelinessState::StoppedUncleanly;

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
        assert_eq!(building, building_clone);
    }

    #[test]
    fn test_graph_liveliness_state_debug_output() {
        let building = GraphLivelinessState::Building;
        let debug_str = format!("{:?}", building);
        assert_eq!(debug_str, "Building");
    }
}
