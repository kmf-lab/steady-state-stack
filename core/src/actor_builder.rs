//! The `actor_builder` module provides structures and functions to create, configure, and manage actors within a system.
//! This module includes the `ActorBuilder` for building actors, `Troupe` for managing groups of actors, and various utility
//! functions and types to support actor creation and telemetry monitoring.

use std::any::Any;
use std::error::Error;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use core::default::Default;
use std::collections::VecDeque;
use futures::channel::oneshot;
use futures::channel::oneshot::{Receiver, Sender};
use futures_util::lock::{Mutex, MutexGuard};
#[allow(unused_imports)]
use log::*;
use futures_util::future::select_all;
use crate::*;
use crate::{steady_config, ActorName, AlertColor, Graph, StdDev, Trigger};
use crate::graph_liveliness::{ActorIdentity, GraphLiveliness};
use crate::graph_testing::{SideChannel, StageManager};
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use aeron::aeron::Aeron;
use async_lock::Barrier;
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::dot::RemoteDetails;

/// The `ActorBuilder` struct is responsible for constructing and configuring actors within the system.
/// It provides a fluent interface to set various properties and behaviors of the actor, such as telemetry settings,
/// trigger conditions, and execution parameters. Once configured, the builder can spawn the actor either standalone
/// or as part of a `Troupe`.
#[derive(Clone)]
pub struct ActorBuilder {
    /// The name of the actor, used for identification in telemetry and logging.
    actor_name: ActorName,
    /// Shared arguments passed to the actor, accessible via the `args` method in `SteadyContext`.
    args: Arc<Box<dyn Any + Send + Sync>>,
    /// Telemetry transmitter for collecting and sending actor metrics.
    telemetry_tx: Arc<RwLock<Vec<CollectorDetail>>>,
    /// Shared counter for the number of channels in the graph.
    channel_count: Arc<AtomicUsize>,
    /// Shared liveliness state of the graph, used for managing actor lifecycle.
    runtime_state: Arc<RwLock<GraphLiveliness>>,
    /// Shared counter for the number of actors in the graph.
    actor_count: Arc<AtomicUsize>,
    /// Mutex for synchronizing thread operations, particularly for core affinity settings.
    thread_lock: Arc<Mutex<()>>,
    /// List of CPU cores to exclude from actor assignment.
    excluded_cores: Vec<usize>,
    /// Optional core balancer for distributing actors across available cores.
    core_balancer: Option<CoreBalancer>,
    /// Optional explicit core assignment for the actor.
    explicit_core: Option<usize>,
    /// Bit shift value determining the refresh rate for telemetry data.
    refresh_rate_in_bits: u8,
    /// Bit shift value determining the window bucket size for metrics aggregation.
    window_bucket_in_bits: u8,
    /// Flag indicating whether usage review is enabled for the actor.
    usage_review: bool,
    /// Percentiles to monitor for CPU usage metrics.
    percentiles_mcpu: Vec<Percentile>,
    /// Percentiles to monitor for workload metrics.
    percentiles_load: Vec<Percentile>,
    /// Standard deviations to monitor for CPU usage metrics.
    std_dev_mcpu: Vec<StdDev>,
    /// Standard deviations to monitor for workload metrics.
    std_dev_load: Vec<StdDev>,
    /// Triggers for CPU usage that raise alerts with associated colors.
    trigger_mcpu: Vec<(Trigger<MCPU>, AlertColor)>,
    /// Triggers for workload that raise alerts with associated colors.
    trigger_load: Vec<(Trigger<Work>, AlertColor)>,
    /// Flag indicating whether to include thread information in telemetry data.
    show_thread_info: bool,
    /// Flag indicating whether to monitor average CPU usage.
    avg_mcpu: bool,
    /// Flag indicating whether to monitor average workload.
    avg_load: bool,
    /// Frame rate in milliseconds for telemetry data collection.
    frame_rate_ms: u64,
    /// Shared vector of oneshot senders for shutdown notifications.
    oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    /// Backplane for side-channel communications, primarily used in testing.
    backplane: Arc<Mutex<Option<StageManager>>>,
    /// Shared counter for the number of actor teams.
    team_count: Arc<AtomicUsize>,
    /// Optional details for remote communication in distributed systems.
    remote_details: Option<RemoteDetails>,
    /// Flag indicating whether to prevent simulation, ensuring real execution.
    pub(crate) never_simulate: bool,
    /// Lazily initialized Aeron media driver for communication.
    aeron_media_driver: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    /// Optional barrier for synchronizing actor shutdown.
    pub shutdown_barrier: Option<Arc<Barrier>>,
    /// Flag indicating whether the actor is for testing purposes.
    is_for_test: bool,
    /// Optional stack size for the actor.
    stack_size: Option<usize>,
}

/// A helper struct for managing CPU core allocation to balance actor distribution across available cores.
///
/// `CoreBalancer` tracks the usage of each core and allocates actors to the least utilized cores, respecting any
/// exclusions specified in the `ActorBuilder`.
#[derive(Clone)]
pub struct CoreBalancer {
    /// A vector where each element represents the number of actors assigned to that core.
    core_usage: Vec<usize>,
}

impl CoreBalancer {
    /// Allocates a core for an actor, choosing the least utilized core that is not excluded.
    ///
    /// # Arguments
    ///
    /// * `excluded_cores` - A slice of core indices to exclude from allocation.
    ///
    /// # Returns
    ///
    /// The index of the allocated core.
    fn allocate_core(&mut self, excluded_cores: &[usize]) -> usize {
        let core = self
            .core_usage
            .iter()
            .enumerate()
            .filter(|(i, _)| !excluded_cores.contains(i))
            .min_by_key(|(_, count)| *count)
            .map(|(core, _)| core)
            .expect("No available cores");
        self.core_usage[core] += 1;
        core
    }
}

/// Retrieves the number of available CPU cores on Unix systems.
///
/// # Returns
///
/// The number of CPU cores available.
#[cfg(feature = "core_affinity")]
#[cfg(unix)]
fn get_num_cores() -> usize {
    unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) as usize }
}

/// Pins the current thread to a specific CPU core.
///
/// # Arguments
///
/// * `core_id` - The index of the core to pin the thread to.
///
/// # Returns
///
/// A `Result` indicating success or an error message if pinning fails.
#[cfg(feature = "core_affinity")]
fn pin_thread_to_core(core_id: usize) -> Result<(), String> {
    #[cfg(unix)]
    {
        let num_cores = get_num_cores();
        let core_id = core_id % num_cores;
        let mut cpu_set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::CPU_ZERO(&mut cpu_set);
            libc::CPU_SET(core_id, &mut cpu_set);
            let thread_id = libc::pthread_self();
            let result = libc::pthread_setaffinity_np(
                thread_id,
                std::mem::size_of::<libc::cpu_set_t>(),
                &cpu_set,
            );
            if result != 0 {
                return Err(format!("Failed to set thread affinity: {}", result));
            }
        }
    }
    #[cfg(windows)]
    {
        unsafe {
            let thread = winapi::um::processthreadsapi::GetCurrentThread();
            let mask = 1usize << core_id;
            winapi::um::winbase::SetThreadAffinityMask(thread, mask);
        }
    }
    Ok(())
}

/// Manages a collection of actors, facilitating their coordinated execution on a shared thread.
///
/// `Troupe` allows grouping multiple actors to run concurrently on the same thread, improving efficiency by reducing
/// thread management overhead.
pub struct Troupe {
    /// A queue of future builders for the actors in the troupe.
    future_builder: VecDeque<FutureBuilderType>,
    /// Unique identifier for the troupe.
    team_id: usize,
}

/// A type alias for a pinned future representing an actor's execution logic.
pub type PinnedFuture = Pin<Box<dyn Future<Output = Result<(), Box<dyn Error>>> + 'static>>;

/// A type alias for a dynamic function that takes a `SteadyActorShadow` and returns a `PinnedFuture`.
pub type DynCall = Box<dyn Fn(SteadyActorShadow) -> PinnedFuture + 'static>;

/// A type alias for the runtime representation of an actor's execution logic, wrapped to avoid `Send` requirements.
type ActorRuntime = NonSendWrapper<DynCall>;

/// Represents a builder for a future, encapsulating the actor's execution logic and execution parameters.
struct FutureBuilderType {
    /// The archetype containing the actor's execution logic and context.
    fun: SteadyContextArchetype<DynCall>,
    /// The frame rate in milliseconds for telemetry data collection.
    frame_rate_ms: u64,
    /// Flag indicating whether the actor is for testing purposes.
    is_for_test: bool,
    /// Optional stack size for the actor.
    stack_size: Option<usize>,
}

impl FutureBuilderType {
    /// Creates a new `FutureBuilderType` instance.
    ///
    /// # Arguments
    ///
    /// * `fun` - The archetype containing the actor's execution logic and context.
    /// * `frame_rate_ms` - The frame rate in milliseconds for telemetry data collection.
    /// * `is_for_test` - Flag indicating whether the actor is for testing purposes.
    /// * `stack_size` - Optional stack size for the actor.
    ///
    /// # Returns
    ///
    /// A new `FutureBuilderType` instance.
    fn new(fun: SteadyContextArchetype<DynCall>, frame_rate_ms: u64, is_for_test: bool, stack_size: Option<usize>) -> Self {
        FutureBuilderType {
            fun,
            frame_rate_ms,
            is_for_test,
            stack_size,
        }
    }

    /// Registers the actor with the graph's liveliness state and returns the execution logic wrapper.
    ///
    /// # Returns
    ///
    /// The `ActorRuntime` containing the registered execution logic.
    fn register(&self) -> ActorRuntime {
        build_actor_registration(&self.fun)
    }

    /// Constructs a `SteadyActorShadow` context for the actor.
    ///
    /// # Arguments
    ///
    /// * `team_display_id` - The identifier of the team for display purposes.
    ///
    /// # Returns
    ///
    /// A `SteadyActorShadow` instance representing the actor's runtime context.
    fn context(&self, team_display_id: usize) -> SteadyActorShadow {
        build_actor_context(&self.fun, self.frame_rate_ms, team_display_id, self.is_for_test)
    }
}

/// A guard that automatically spawns the troupe when it goes out of scope.
///
/// This guard ensures that the troupe is spawned only when the guard is dropped, allowing for deferred execution.
pub struct TroupeGuard {
    /// The optional troupe to be spawned when the guard is dropped.
    pub(crate) troupe: Option<Troupe>,
}

impl std::ops::Deref for TroupeGuard {
    type Target = Troupe;

    /// Provides immutable access to the underlying troupe.
    fn deref(&self) -> &Self::Target {
        self.troupe
            .as_ref()
            .expect("TroupeGuard troupe was already consumed")
    }
}

impl std::ops::DerefMut for TroupeGuard {
    /// Provides mutable access to the underlying troupe.
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.troupe
            .as_mut()
            .expect("TroupeGuard troupe was already consumed")
    }
}

impl Drop for TroupeGuard {
    /// Spawns the troupe when the guard is dropped, initiating the execution of the actors.
    fn drop(&mut self) {
        if let Some(troupe) = self.troupe.take() {
            troupe.spawn();
        }
    }
}

impl Troupe {
    /// Creates a new `Troupe` instance with a unique team identifier derived from the graph.
    ///
    /// # Arguments
    ///
    /// * `graph` - A reference to the `Graph` from which to derive the team count.
    ///
    /// # Returns
    ///
    /// A new `Troupe` instance.
    pub(crate) fn new(graph: &Graph) -> Self {
        Troupe {
            future_builder: VecDeque::new(),
            team_id: graph.team_count.fetch_add(1, Ordering::SeqCst),
        }
    }

    /// Adds an actor to the troupe with the specified context and execution parameters.
    ///
    /// # Arguments
    ///
    /// * `context_archetype` - The archetype containing the actor's execution logic and context.
    /// * `frame_rate_ms` - The frame rate in milliseconds for telemetry data collection.
    /// * `is_for_test` - Flag indicating whether the actor is for testing purposes.
    /// * `stack_size` - Optional stack size for the actor.
    fn add_actor(
        &mut self,
        context_archetype: SteadyContextArchetype<DynCall>,
        frame_rate_ms: u64,
        is_for_test: bool,
        stack_size: Option<usize>,
    ) {
        self.future_builder.push_back(FutureBuilderType::new(
            context_archetype.clone(),
            frame_rate_ms,
            is_for_test,
            stack_size,
        ));
    }

    /// Transfers the front actor to another `Troupe`.
    ///
    /// # Arguments
    ///
    /// * `other` - The target `Troupe` to receive the actor.
    ///
    /// # Returns
    ///
    /// `true` if an actor was transferred, `false` if the troupe is empty.
    pub fn transfer_front_to(&mut self, other: &mut Self) -> bool {
        if let Some(f) = self.future_builder.pop_front() {
            other.future_builder.push_back(f);
            true
        } else {
            false
        }
    }

    /// Transfers the back actor to another `Troupe`.
    ///
    /// # Arguments
    ///
    /// * `other` - The target `Troupe` to receive the actor.
    ///
    /// # Returns
    ///
    /// `true` if an actor was transferred, `false` if the troupe is empty.
    pub fn transfer_back_to(&mut self, other: &mut Self) -> bool {
        if let Some(f) = self.future_builder.pop_back() {
            other.future_builder.push_back(f);
            true
        } else {
            false
        }
    }

    /// Spawns the troupe, executing all actors on a shared thread.
    ///
    /// # Returns
    ///
    /// The number of actors spawned.
    fn spawn(mut self) -> usize {
        let count = Arc::new(AtomicUsize::new(0));
        if self.future_builder.is_empty() {
            return 0;
        }

        let (local_send, local_take) = oneshot::channel();
        let count_task = count.clone();
        let team_id = self.team_id;
        let max_stack_size = self.future_builder.iter().filter_map(|f| f.stack_size).max();

        let super_task = async move {
            #[cfg(feature = "core_affinity")]
            {
                let core = team_id;
                if let Err(e) = pin_thread_to_core(core) {
                    eprintln!("Failed to pin thread to core {}: {:?}", core, e);
                }
            }
            let double_vec: Vec<(ActorRuntime, bool)> = self
                .future_builder
                .iter_mut()
                .map(|f| (f.register(), false))
                .collect();

            count_task.store(double_vec.len(), Ordering::SeqCst);
            let _ = local_send.send(());

            let triplet_vec: Vec<(ActorRuntime, SteadyActorShadow, bool)> = self
                .future_builder
                .iter()
                .zip(double_vec)
                .map(|(f, (e, b))| (e, f.context(team_id), b))
                .collect();

            let actor_future_vec: Vec<_> = triplet_vec
                .iter()
                .map(|(fun, ctx, _drive_io)| {
                    let ctx = ctx.clone();
                    Self::build_async_fun(fun, ctx)
                })
                .collect();

            let mut future_all = select_all(actor_future_vec);
            loop {
                let result = catch_unwind(AssertUnwindSafe(|| launch_actor(&mut future_all)));
                match result {
                    Ok((actor_result, index, mut leftover_futures)) => {
                        let (fun, ctx, _drive_io) = &triplet_vec[index];
                        if let Err(e) = actor_result {
                            error!("Actor at index {index} got error: {e:?}");
                            let ctx = ctx.clone();
                            leftover_futures.insert(index, Self::build_async_fun(fun, ctx));
                            continue;
                        }
                        if leftover_futures.get(index).is_some() {
                            drop(core_exec::block_on(leftover_futures.remove(index)));
                        }
                        exit_actor_registration(&self.future_builder[index].fun);
                        if leftover_futures.is_empty() {
                            break;
                        }
                        future_all = select_all(leftover_futures);
                    }
                    Err(e) => {
                        error!("Actor panic: {e:?}");
                        let actor_future_vec: Vec<_> = triplet_vec
                            .iter()
                            .map(|(fun, ctx, _drive_io)| {
                                let ctx = ctx.clone();
                                Self::build_async_fun(fun, ctx)
                            })
                            .collect();
                        future_all = select_all(actor_future_vec);
                    }
                }
            }
        };
        core_exec::block_on(async move {
            //two so we can support blocking calls.
            match core_exec::spawn_more_threads(2).await {
                Ok(c) => {
                    if c >= 12 {
                        info!("Threads: {}", c);
                    }
                }
                Err(e) => {
                    error!("Failed to spawn one more thread: {:?}", e);
                }
            }
            
            let mut thread_builder = std::thread::Builder::new().name(format!("Troupe-{}", team_id));
            if let Some(size) = max_stack_size {
                thread_builder = thread_builder.stack_size(size);
            }
            let handle = thread_builder.spawn(move || {
                core_exec::block_on(super_task);
            });
            if let Err(e) = handle {
                error!("Failed to spawn OS thread for troupe: {}, error: {:?}", team_id, e);
            }
        });
        let _ = core_exec::block_on(local_take);
        count.load(Ordering::SeqCst)
    }

    /// Builds an asynchronous function for an actor's execution logic.
    ///
    /// # Arguments
    ///
    /// * `fun` - The runtime representation of the actor's execution logic.
    /// * `ctx` - The `SteadyActorShadow` context for the actor.
    ///
    /// # Returns
    ///
    /// a pinned future representing the actor's execution.
    fn build_async_fun(
        fun: &ActorRuntime,
        mut ctx: SteadyActorShadow,
    ) -> Pin<Box<impl Future<Output = Result<(), Box<dyn Error>>> + Sized + '_>> {
        Box::pin(async move {
            let guard_fun = fun.lock().await;
            ctx.regeneration +=1; //restart counts when in a troupe
            guard_fun(ctx.clone()).await
        })
    }
}

/// Launches an actor by blocking on its future until completion.
///
/// **Warning:** Do not rename this function without updating backtrace printing, as it serves as a "stop" to shorten traces.
///
/// # Type Parameters
///
/// * `F` - The type of the future to execute.
/// * `T` - The output type of the future.
///
/// # Arguments
///
/// * `future` - The future to execute.
///
/// # Returns
///
/// The result of the future execution.
pub fn launch_actor<F: Future<Output = T>, T>(future: F) -> T {
    core_exec::block_on(future)
}

/// A type alias for a mutex containing a side-channel transmitter and shutdown receiver, used in testing.
pub(crate) type NodeTxRx = Mutex<(SideChannel, Receiver<()>)>;

/// A template for building actor contexts, encapsulating all necessary parameters and state for actor execution.
///
/// This struct serves as a blueprint for creating `SteadyActorShadow` instances, which provide the runtime environment
/// for actors.
struct SteadyContextArchetype<DynCall: ?Sized> {
    /// The execution logic for the actor, wrapped to avoid `Send` requirements.
    build_actor_exec: NonSendWrapper<DynCall>,
    /// Shared liveliness state of the graph.
    runtime_state: Arc<RwLock<GraphLiveliness>>,
    /// Shared counter for the number of channels.
    channel_count: Arc<AtomicUsize>,
    /// Unique identifier for the actor.
    ident: ActorIdentity,
    /// Shared arguments for the actor.
    args: Arc<Box<dyn Any + Send + Sync>>,
    /// Telemetry receivers for monitoring.
    all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    /// Metadata for the actor, including telemetry configurations.
    actor_metadata: Arc<ActorMetaData>,
    /// Vector of oneshot senders for shutdown notifications.
    oneshot_shutdown_vec: Arc<Mutex<Vec<Sender<()>>>>,
    /// Oneshot receiver for shutdown signals.
    oneshot_shutdown: Arc<Mutex<Receiver<()>>>,
    /// Optional node transmitter and receiver for side-channel communications.
    node_tx_rx: Option<Arc<NodeTxRx>>,
    /// Flag indicating whether to show thread information in telemetry.
    show_thread_info: bool,
    /// Lazily initialized Aeron media driver.
    aeron_media_driver: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    /// Flag indicating whether to prevent simulation.
    never_simulate: bool,
    /// Optional barrier for synchronizing shutdown.
    shutdown_barrier: Option<Arc<Barrier>>,
}

impl<T: ?Sized> Clone for SteadyContextArchetype<T> {
    fn clone(&self) -> Self {
        SteadyContextArchetype {
            build_actor_exec: self.build_actor_exec.clone(),
            runtime_state: self.runtime_state.clone(),
            channel_count: self.channel_count.clone(),
            ident: self.ident,
            args: self.args.clone(),
            all_telemetry_rx: self.all_telemetry_rx.clone(),
            actor_metadata: self.actor_metadata.clone(),
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown: self.oneshot_shutdown.clone(),
            node_tx_rx: self.node_tx_rx.clone(),
            show_thread_info: self.show_thread_info,
            aeron_media_driver: self.aeron_media_driver.clone(),
            never_simulate: self.never_simulate,
            shutdown_barrier: self.shutdown_barrier.clone(),
        }
    }
}

/// Represents the scheduling options for an actor, either as a solo act or a member of a troupe.
pub enum ScheduleAs<'a> {
    /// The actor runs independently on its own thread.
    SoloAct,
    /// The actor is part of a troupe, sharing a thread with other actors.
    MemberOf(&'a mut Troupe),
}

impl ScheduleAs<'_> {
    /// Determines the scheduling type based on the presence of a troupe guard.
    ///
    /// # Arguments
    ///
    /// * `some_troupe` - An optional troupe guard to check.
    ///
    /// # Returns
    ///
    /// The appropriate `ScheduleAs` variant.
    pub fn dynamic_schedule(some_troupe: &mut Option<TroupeGuard>) -> ScheduleAs<'_> {
        if let Some(t) = some_troupe {
            ScheduleAs::MemberOf(t)
        } else {
            ScheduleAs::SoloAct
        }
    }
}

impl ActorBuilder {
    /// Creates a new `ActorBuilder` instance, initializing it with default settings derived from the given `Graph`.
    ///
    /// This method sets up the builder with configurations inherited from the graph, such as telemetry settings and
    /// liveliness state. It computes default values for the refresh rate and window bucket size based on the graph's
    /// telemetry production rate.
    ///
    /// # Arguments
    ///
    /// * `graph` - A mutable reference to the `Graph` from which to inherit settings.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance configured with the graph's settings.
    pub fn new(graph: &mut Graph) -> ActorBuilder {
        let (refresh_in_bits, window_in_bits) = ActorBuilder::internal_compute_refresh_window(
            graph.telemetry_production_rate_ms as u128,
            Duration::from_secs(1),
            Duration::from_secs(10),
        );
        ActorBuilder {
            actor_name: ActorName::new("", None),
            backplane: graph.backplane.clone(),
            thread_lock: graph.thread_lock.clone(),
            excluded_cores: vec![],
            actor_count: graph.actor_count.clone(),
            args: graph.args.clone(),
            telemetry_tx: graph.all_telemetry_rx.clone(),
            channel_count: graph.channel_count.clone(),
            runtime_state: graph.runtime_state.clone(),
            refresh_rate_in_bits: refresh_in_bits,
            window_bucket_in_bits: window_in_bits,
            oneshot_shutdown_vec: graph.oneshot_shutdown_vec.clone(),
            percentiles_mcpu: Vec::with_capacity(0),
            percentiles_load: Vec::with_capacity(0),
            std_dev_mcpu: Vec::with_capacity(0),
            std_dev_load: Vec::with_capacity(0),
            trigger_mcpu: Vec::with_capacity(0),
            trigger_load: Vec::with_capacity(0),
            team_count: graph.team_count.clone(),
            explicit_core: None,
            show_thread_info: false,
            avg_mcpu: false,
            avg_load: false,
            frame_rate_ms: graph.telemetry_production_rate_ms,
            usage_review: false,
            core_balancer: None,
            remote_details: None,
            never_simulate: false,
            aeron_media_driver: graph.aeron.clone(),
            shutdown_barrier: graph.shutdown_barrier.clone(),
            is_for_test: graph.is_for_testing,
            stack_size: graph.default_stack_size,
        }
    }

    /// Sets the compute refresh window floor and bucket size for telemetry, adjusting the resolution of performance metrics.
    ///
    /// This method fine-tunes telemetry data collection by specifying the minimum refresh rate and window size for
    /// metrics aggregation.
    ///
    /// # Arguments
    ///
    /// * `refresh` - The minimum refresh rate as a `Duration`.
    /// * `window` - The size of the window as a `Duration`.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the updated compute refresh window configuration.
    pub fn with_compute_refresh_window_floor(&self, refresh: Duration, window: Duration) -> Self {
        let mut result = self.clone();
        let (refresh_in_bits, window_in_bits) =
            ActorBuilder::internal_compute_refresh_window(self.frame_rate_ms as u128, refresh, window);
        result.refresh_rate_in_bits = refresh_in_bits;
        result.window_bucket_in_bits = window_in_bits;
        result
    }

    /// Configures the actor to exclude specific CPU cores from being assigned to it.
    ///
    /// This is useful for avoiding cores reserved for other tasks or balancing system load.
    ///
    /// # Arguments
    ///
    /// * `cores` - A vector of core indices to exclude.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified core exclusions.
    pub fn with_core_exclusion(&self, cores: Vec<usize>) -> Self {
        let mut result = self.clone();
        result.excluded_cores = cores;
        result
    }

    /// Configures the actor to use a core balancer for dynamic core allocation.
    ///
    /// The core balancer distributes actors across available cores to optimize resource usage.
    ///
    /// # Arguments
    ///
    /// * `balancer` - An instance of `CoreBalancer` for core allocation.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified core balancer.
    pub fn with_core_balancing(&self, balancer: CoreBalancer) -> Self {
        let mut result = self.clone();
        result.core_balancer = Some(balancer);
        result
    }

    /// Assigns the actor to a specific CPU core explicitly, overriding any balancing or default assignment.
    ///
    /// # Arguments
    ///
    /// * `zero_offset_core` - The zero-based index of the core to assign the actor to.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the explicit core assignment.
    pub fn with_explicit_core(&self, one_offset_core: u16) -> Self {
        let mut result = self.clone();
        assert!(one_offset_core > 0, "Core index must be greater than zero and match your OS task manager.");
        let zero_offset_core = one_offset_core - 1;
        result.explicit_core = Some(zero_offset_core.into());
        result
    }

    /// Disables telemetry metric collection for the actor, useful for performance-critical scenarios.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with telemetry disabled.
    pub fn with_no_refresh_window(&self) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = 0;
        result.window_bucket_in_bits = 0;
        result
    }

    /// Computes the refresh rate and window bucket size in bits based on frame rate and durations.
    ///
    /// # Arguments
    ///
    /// * `frame_rate_ms` - The frame rate in milliseconds.
    /// * `refresh` - The desired refresh duration.
    /// * `window` - The desired window duration.
    ///
    /// # Returns
    ///
    /// A tuple of `(refresh_in_bits, window_in_bits)` representing the computed values.
    pub(crate) fn internal_compute_refresh_window(
        frame_rate_ms: u128,
        refresh: Duration,
        window: Duration,
    ) -> (u8, u8) {
        if frame_rate_ms > 0 {
            let frames_per_refresh = refresh.as_micros() / (1000u128 * frame_rate_ms);
            let refresh_in_bits = (frames_per_refresh as f32).log2().ceil() as u8;
            let refresh_in_micros = (1000u128 << refresh_in_bits) * frame_rate_ms;
            let buckets_per_window: f32 = window.as_micros() as f32 / refresh_in_micros as f32;
            let window_in_bits = buckets_per_window.log2().ceil() as u8;
            (refresh_in_bits, window_in_bits)
        } else {
            (0, 0)
        }
    }

    /// Configures the actor to monitor a specific CPU usage percentile for performance analysis.
    ///
    /// # Arguments
    ///
    /// * `config` - The `Percentile` to monitor for CPU usage.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified CPU usage percentile.
    pub fn with_mcpu_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_mcpu.push(config);
        result
    }

    /// Sets the actor's name with a suffix for telemetry identification.
    ///
    /// # Arguments
    ///
    /// * `name` - The base name of the actor.
    /// * `suffix` - A numeric suffix for uniqueness.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified name and suffix.
    pub fn with_name_and_suffix(&self, name: &'static str, suffix: usize) -> Self {
        let mut result = self.clone();
        result.actor_name = ActorName::new(name, Some(suffix));
        result
    }

    /// Sets the actor's name for telemetry identification.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the actor.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified name.
    pub fn with_name(&self, name: &'static str) -> Self {
        let mut result = self.clone();
        result.actor_name = ActorName::new(name, None);
        result
    }

    /// Configures whether the actor should never be simulated, ensuring real execution.
    ///
    /// # Arguments
    ///
    /// * `never_simulate` - Flag to prevent simulation.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the simulation setting.
    pub fn never_simulate(&self, never_simulate: bool) -> Self {
        let mut result = self.clone();
        result.never_simulate = never_simulate;
        result
    }

    /// Configures the actor to monitor a specific workload percentile for performance analysis.
    ///
    /// # Arguments
    ///
    /// * `config` - The `Percentile` to monitor for workload.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified workload percentile.
    pub fn with_load_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_load.push(config);
        result
    }

    /// Enables average CPU usage monitoring for the actor.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with average CPU monitoring enabled.
    pub fn with_mcpu_avg(&self) -> Self {
        let mut result = self.clone();
        result.avg_mcpu = true;
        result
    }

    /// Enables average workload monitoring for the actor.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with average workload monitoring enabled.
    pub fn with_load_avg(&self) -> Self {
        let mut result = self.clone();
        result.avg_load = true;
        result
    }

    /// Sets a CPU usage trigger that raises an alert when exceeded.
    ///
    /// # Arguments
    ///
    /// * `bound` - The trigger condition based on CPU usage.
    /// * `color` - The `AlertColor` for the alert.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the CPU trigger.
    pub fn with_mcpu_trigger(&self, bound: Trigger<MCPU>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_mcpu.push((bound, color));
        result
    }

    /// Sets a workload trigger that raises an alert when exceeded.
    ///
    /// # Arguments
    ///
    /// * `bound` - The trigger condition based on workload.
    /// * `color` - The `AlertColor` for the alert.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the workload trigger.
    pub fn with_load_trigger(&self, bound: Trigger<Work>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_load.push((bound, color));
        result
    }

    /// Configures the actor with remote communication details for distributed systems.
    ///
    /// # Arguments
    ///
    /// * `ip_vec` - Vector of IP addresses.
    /// * `match_on` - String to match for communication.
    /// * `is_input` - Flag indicating input or output direction.
    /// * `tech` - Technology identifier for communication.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with remote details.
    pub(crate) fn with_remote_details(
        &self,
        ip_vec: Vec<String>,
        match_on: String,
        is_input: bool,
        tech: &'static str,
    ) -> Self {
        let mut result = self.clone();
        result.remote_details = Some(RemoteDetails {
            ips: ip_vec.join(","),
            match_on,
            tech,
            direction: if is_input { "in" } else { "out" },
        });
        result
    }

    /// Enables thread information in telemetry data.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with thread info enabled.
    pub fn with_thread_info(&self) -> Self {
        let mut result = self.clone();
        result.show_thread_info = true;
        result
    }

    /// Sets the stack size for the actor.
    ///
    /// # Arguments
    ///
    /// * `mb` - The desired stack size in megabytes.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the updated stack size.
    pub fn with_stack_size(&self, mb: usize) -> Self {
        let mut result = self.clone();
        result.stack_size = Some(mb * 1024 * 1024);
        result
    }

    /// Completes the actor configuration and spawns it with the provided execution logic.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic function.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - The execution logic for the actor.
    fn build_spawn<F, I>(self, build_actor_exec: I)
    where
        I: Fn(SteadyActorShadow) -> F + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        let excluded_cores = self.excluded_cores.clone();
        let core_balancer = self.core_balancer.clone();
        let explicit_core = self.explicit_core;
        let default_core = self.team_count.clone().fetch_add(1, Ordering::SeqCst);
        let thread_lock = self.thread_lock.clone();
        let rate_ms = self.frame_rate_ms;
        let is_for_test = self.is_for_test;
        let actor_name = self.actor_name.clone();
        let stack_size = self.stack_size;

        let context_archetype = self.clone().single_actor_exec_archetype(build_actor_exec);

        core_exec::block_on(async move {
            let _guard = thread_lock.lock().await;

            let fun: NonSendWrapper<DynCall> = build_actor_registration(&context_archetype);
            let mut master_ctx: SteadyActorShadow =
                build_actor_context(&context_archetype, rate_ms, default_core, is_for_test);

            let actor_name_clone = actor_name.name;
            
            let mut thread_builder = std::thread::Builder::new().name(actor_name_clone.to_string());
            if let Some(size) = stack_size {
                thread_builder = thread_builder.stack_size(size);
            }
            
            let handle = thread_builder.spawn(move || {
                    let default = if let Some(exp) = explicit_core {
                        exp
                    } else {
                        default_core
                    };
                    let _core = if let Some(mut balancer) = core_balancer {
                        balancer.allocate_core(&excluded_cores)
                    } else if !excluded_cores.is_empty() {
                        if !excluded_cores.contains(&default) {
                            default
                        } else {
                            (0..excluded_cores.len())
                                .find(|&core| !excluded_cores.contains(&core))
                                .unwrap_or(default)
                        }
                    } else {
                        default
                    };

                    #[cfg(feature = "core_affinity")]
                    {
                        if let Err(e) = pin_thread_to_core(_core) {
                            eprintln!("Failed to pin thread to core {}: {:?}", _core, e);
                        }
                    }
                    
                    info!("Spawning SoloAct {:?} on new OS thread", &actor_name_clone);

                    loop {
                        match catch_unwind(AssertUnwindSafe(|| match fun.clone().try_lock() {
                            Some(actor_run) => launch_actor(actor_run(master_ctx.clone())),
                            None => panic!("internal error, future (actor) already locked"),
                        })) {
                            Ok(_) => {
                                exit_actor_registration(&context_archetype);
                                break;
                            }
                            Err(e) => {
                                if let Some(specific_error) = e.downcast_ref::<std::io::Error>() {
                                    warn!(
                                        "IO Error encountered: {} in actor: {:?}",
                                        specific_error, context_archetype.ident
                                    );
                                } else if let Some(specific_error) = e.downcast_ref::<String>() {
                                    warn!(
                                        "String Error encountered: {} in actor: {:?}",
                                        specific_error, context_archetype.ident
                                    );
                                }
                                master_ctx.regeneration += 1;
                                warn!("Restarting: {:?}", context_archetype.ident);
                            }
                        }
                    }
                });

            if let Err(e) = handle {
                error!("Failed to spawn OS thread for actor: {:?}, error: {:?}", &self.actor_name.name, e);
            }
        });
    }

    /// Adds an actor to the specified `Troupe` for group execution.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic function.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - The execution logic for the actor.
    /// * `target` - The `Troupe` to add the actor to.
    fn build_join<F, I>(self, build_actor_exec: I, target: &mut Troupe)
    where
        I: Fn(SteadyActorShadow) -> F + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        let rate = self.frame_rate_ms;
        let is_for_test = self.is_for_test;
        let stack_size = self.stack_size;
        let temp: SteadyContextArchetype<DynCall> = self.single_actor_exec_archetype(build_actor_exec);
        target.add_actor(temp, rate, is_for_test, stack_size);
    }

    /// Builds and schedules an actor based on the desired scheduling type.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic function.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - The execution logic for the actor.
    /// * `desired_scheduling` - The scheduling type (`SoloAct` or `MemberOf`).
    pub fn build<F, I>(self, build_actor_exec: I, desired_scheduling: ScheduleAs)
    where
        I: Fn(SteadyActorShadow) -> F + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        match desired_scheduling {
            ScheduleAs::SoloAct => {
                self.build_spawn(build_actor_exec);
            }
            ScheduleAs::MemberOf(team) => {
                self.build_join(build_actor_exec, team);
            }
        }
    }

    /// Converts a generic function into a dynamic callable object.
    ///
    /// # Type Parameters
    ///
    /// * `I` - The input function type.
    /// * `F` - The future type returned by the function.
    ///
    /// # Arguments
    ///
    /// * `f` - The function to convert.
    ///
    /// # Returns
    ///
    /// A boxed dynamic function compatible with `DynCall`.
    fn to_dyn_call<I, F>(f: I) -> Box<dyn Fn(SteadyActorShadow) -> PinnedFuture>
    where
        I: Fn(SteadyActorShadow) -> F + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        Box::new(move |ctx| Pin::from(Box::new(f(ctx))))
    }

    /// Creates a `SteadyContextArchetype` for actor execution with the specified logic.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic function.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - The execution logic for the actor.
    ///
    /// # Returns
    ///
    /// A `SteadyContextArchetype` configured with the actor's execution logic.
    fn single_actor_exec_archetype<F, I>(self, build_actor_exec: I) -> SteadyContextArchetype<DynCall>
    where
        I: Fn(SteadyActorShadow) -> F + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        let telemetry_tx = self.telemetry_tx.clone();
        let channel_count = self.channel_count.clone();
        let runtime_state = self.runtime_state.clone();
        let args = self.args.clone();
        let oneshot_shutdown_vec = self.oneshot_shutdown_vec.clone();
        let backplane = self.backplane.clone();
        let id = self.actor_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dyn_call = Self::to_dyn_call(build_actor_exec);
        let immutable_identity = ActorIdentity::new(id, self.actor_name.name, self.actor_name.suffix);
        if steady_config::SHOW_ACTORS {
            info!(" Actor {:?} defined ", immutable_identity);
        }
        let immutable_actor_metadata = self.build_actor_metadata(immutable_identity).clone();
        let oneshot_shutdown_vec_for_node = oneshot_shutdown_vec.clone();
        let immutable_node_tx_rx = core_exec::block_on(async move {
            let mut backplane = backplane.lock().await;
            if let Some(pb) = &mut *backplane {
                let (shutdown_tx, shutdown_rx) = oneshot::channel();
                core_exec::block_on(async move {
                    let mut v = oneshot_shutdown_vec_for_node.lock().await;
                    v.push(shutdown_tx);
                });
                pb.register_node(immutable_identity.label, steady_config::BACKPLANE_CAPACITY, shutdown_rx);
                pb.node_tx_rx(immutable_identity.label)
            } else {
                None
            }
        });
        let immutable_oneshot_shutdown = {
            let (send_shutdown_notice_to_periodic_wait, oneshot_shutdown) = oneshot::channel();
            let oneshot_shutdown_vec = oneshot_shutdown_vec.clone();
            core_exec::block_on(async move {
                let mut v = oneshot_shutdown_vec.lock().await;
                v.push(send_shutdown_notice_to_periodic_wait);
            });
            Arc::new(Mutex::new(oneshot_shutdown))
        };
        SteadyContextArchetype {
            runtime_state: runtime_state.clone(),
            channel_count: channel_count.clone(),
            ident: immutable_identity,
            args: args.clone(),
            all_telemetry_rx: telemetry_tx.clone(),
            actor_metadata: immutable_actor_metadata.clone(),
            oneshot_shutdown_vec: oneshot_shutdown_vec.clone(),
            oneshot_shutdown: immutable_oneshot_shutdown.clone(),
            node_tx_rx: immutable_node_tx_rx.clone(),
            build_actor_exec: NonSendWrapper::new(dyn_call),
            show_thread_info: self.show_thread_info,
            aeron_media_driver: self.aeron_media_driver,
            never_simulate: self.never_simulate,
            shutdown_barrier: self.shutdown_barrier,
        }
    }

    /// Constructs actor metadata for telemetry and monitoring.
    ///
    /// # Arguments
    ///
    /// * `ident` - The unique identifier for the actor.
    ///
    /// # Returns
    ///
    /// An `Arc` containing the actor metadata.
    fn build_actor_metadata(&self, ident: ActorIdentity) -> Arc<ActorMetaData> {
        Arc::new(ActorMetaData {
            ident,
            remote_details: self.remote_details.clone(),
            avg_mcpu: self.avg_mcpu,
            avg_work: self.avg_load,
            percentiles_mcpu: self.percentiles_mcpu.clone(),
            percentiles_work: self.percentiles_load.clone(),
            show_thread_info: self.show_thread_info,
            std_dev_mcpu: self.std_dev_mcpu.clone(),
            std_dev_work: self.std_dev_load.clone(),
            trigger_mcpu: self.trigger_mcpu.clone(),
            trigger_work: self.trigger_load.clone(),
            usage_review: self.usage_review,
            refresh_rate_in_bits: self.refresh_rate_in_bits,
            window_bucket_in_bits: self.window_bucket_in_bits,
        })
    }
}

/// A wrapper to handle types that are not `Send` by using `Arc<Mutex<T>>`.
///
/// This allows non-`Send` types to be used safely in multi-threaded contexts by synchronizing access.
pub struct NonSendWrapper<T: ?Sized> {
    /// The inner value wrapped in an `Arc<Mutex<T>>`.
    inner: Arc<Mutex<T>>,
}

// SAFETY: The wrapper is `Send` because access to `T` is synchronized via `Mutex`.
unsafe impl<T> Send for NonSendWrapper<T> {}

impl<T: ?Sized> NonSendWrapper<T> {
    /// Creates a new `NonSendWrapper` instance with the given inner value.
    ///
    /// # Arguments
    ///
    /// * `inner` - The value to wrap.
    ///
    /// # Returns
    ///
    /// A new `NonSendWrapper` instance.
    pub fn new(inner: T) -> NonSendWrapper<T>
    where
        T: Sized,
    {
        NonSendWrapper {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Asynchronously locks the inner value, providing a mutex guard.
    ///
    /// # Returns
    ///
    /// A `MutexGuard` for accessing the inner value.
    pub async fn lock(&self) -> MutexGuard<'_, T> {
        self.inner.lock().await
    }

    /// Attempts to lock the inner value immediately, returning a guard if successful.
    ///
    /// # Returns
    ///
    /// An `Option` containing a `MutexGuard` if the lock is acquired, or `None` if it is contended.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.inner.try_lock()
    }

    /// Clones the wrapper, providing shared ownership of the inner value.
    ///
    /// # Returns
    ///
    /// A new `NonSendWrapper` instance sharing the same inner value.
    pub fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

/// Registers an actor with the graph's liveliness state and returns its execution logic wrapper.
///
/// # Arguments
///
/// * `builder_source` - The archetype containing the actor's context and logic.
///
/// # Returns
///
/// The `NonSendWrapper` containing the actor's execution logic.
fn build_actor_registration(builder_source: &SteadyContextArchetype<DynCall>) -> NonSendWrapper<DynCall> {
    builder_source
        .runtime_state
        .write()
        .register_voter(builder_source.ident);
    builder_source.build_actor_exec.clone()
}

/// Removes an actor from the graph's liveliness state upon clean exit.
///
/// # Arguments
///
/// * `builder_source` - The archetype containing the actor's context and logic.
fn exit_actor_registration(builder_source: &SteadyContextArchetype<DynCall>) {
    builder_source
        .runtime_state
        .write()
        .remove_voter(builder_source.ident);
}

/// Constructs a `SteadyActorShadow` context for an actor based on the archetype and parameters.
///
/// # Arguments
///
/// * `builder_source` - The archetype containing the actor's context and logic.
/// * `frame_rate_ms` - The frame rate in milliseconds for telemetry.
/// * `team_id` - The identifier of the team the actor belongs to.
/// * `is_test` - Flag indicating if the actor is for testing.
///
/// # Returns
///
/// A `SteadyActorShadow` instance representing the actor's runtime context.
fn build_actor_context<I: ?Sized>(
    builder_source: &SteadyContextArchetype<I>,
    frame_rate_ms: u64,
    team_id: usize,
    is_test: bool,
) -> SteadyActorShadow {
    let uib = builder_source.never_simulate || !is_test;
    SteadyActorShadow {
        runtime_state: builder_source.runtime_state.clone(),
        channel_count: builder_source.channel_count.clone(),
        ident: builder_source.ident,
        args: builder_source.args.clone(),
        all_telemetry_rx: builder_source.all_telemetry_rx.clone(),
        actor_metadata: builder_source.actor_metadata.clone(),
        oneshot_shutdown_vec: builder_source.oneshot_shutdown_vec.clone(),
        oneshot_shutdown: builder_source.oneshot_shutdown.clone(),
        node_tx_rx: builder_source.node_tx_rx.clone(),
        regeneration: 0u32,
        last_periodic_wait: Default::default(),
        is_in_graph: true,
        actor_start_time: Instant::now(),
        team_id,
        frame_rate_ms,
        show_thread_info: builder_source.show_thread_info,
        aeron_meda_driver: builder_source.aeron_media_driver.clone(),
        use_internal_behavior: uib,
        shutdown_barrier: builder_source.shutdown_barrier.clone(),

    }
}

#[cfg(test)]
mod test_actor_builder {
    use crate::{GraphBuilder, GraphLivelinessState, SteadyActor};
    use super::*;

    #[test]
    fn test_core_balancer() {
        let mut cb = CoreBalancer { core_usage: vec![0, 0, 0] };
        assert_eq!(cb.allocate_core(&[]), 0);
        assert_eq!(cb.allocate_core(&[]), 1);
        assert_eq!(cb.allocate_core(&[0]), 2);
        assert_eq!(cb.allocate_core(&[]), 0);
        assert_eq!(cb.core_usage, vec![2, 1, 1]);
    }

    #[test]
    fn test_actor_builder_core_configs() {
        let mut graph = GraphBuilder::for_testing().build(());
        let builder = ActorBuilder::new(&mut graph);
        
        let b2 = builder.with_explicit_core(5);
        assert_eq!(b2.explicit_core, Some(4));

        let b3 = builder.with_core_exclusion(vec![0, 1]);
        assert_eq!(b3.excluded_cores, vec![0, 1]);

        let cb = CoreBalancer { core_usage: vec![0] };
        let b4 = builder.with_core_balancing(cb);
        assert!(b4.core_balancer.is_some());
    }

    #[test]
    #[should_panic]
    fn test_explicit_core_zero_panic() {
        let mut graph = GraphBuilder::for_testing().build(());
        let builder = ActorBuilder::new(&mut graph);
        builder.with_explicit_core(0);
    }

    #[test]
    fn test_troupe_ops() {
        let graph = GraphBuilder::for_testing().build(());
        let mut troupe = Troupe::new(&graph);
        let mut other = Troupe::new(&graph);
        
        // Mock an archetype
        let (_tx, rx) = oneshot::channel();
        let arch = SteadyContextArchetype {
            build_actor_exec: NonSendWrapper::new(ActorBuilder::to_dyn_call(|_| async { Ok(()) })),
            runtime_state: graph.runtime_state.clone(),
            channel_count: graph.channel_count.clone(),
            ident: ActorIdentity::default(),
            args: graph.args.clone(),
            all_telemetry_rx: graph.all_telemetry_rx.clone(),
            actor_metadata: Arc::new(ActorMetaData::default()),
            oneshot_shutdown_vec: graph.oneshot_shutdown_vec.clone(),
            oneshot_shutdown: Arc::new(Mutex::new(rx)),
            node_tx_rx: None,
            show_thread_info: false,
            aeron_media_driver: OnceLock::new(),
            never_simulate: false,
            shutdown_barrier: None,
        };

        troupe.add_actor(arch.clone(), 40, true, None);
        assert_eq!(troupe.future_builder.len(), 1);
        
        assert!(troupe.transfer_front_to(&mut other));
        assert_eq!(troupe.future_builder.len(), 0);
        assert_eq!(other.future_builder.len(), 1);

        assert!(other.transfer_back_to(&mut troupe));
        assert_eq!(troupe.future_builder.len(), 1);
    }

    #[test]
    fn test_schedule_as() {
        let mut troupe_guard = None;
        assert!(matches!(ScheduleAs::dynamic_schedule(&mut troupe_guard), ScheduleAs::SoloAct));
        
        let graph = GraphBuilder::for_testing().build(());
        let mut troupe_guard = Some(graph.actor_troupe());
        assert!(matches!(ScheduleAs::dynamic_schedule(&mut troupe_guard), ScheduleAs::MemberOf(_)));
    }

    #[test]
    fn test_actor_builder_creation_spawn() {
        let mut graph = GraphBuilder::for_testing().build(());
        let builder = ActorBuilder::new(&mut graph);
        assert_eq!(builder.actor_name.name, "");
        assert_eq!(builder.refresh_rate_in_bits, 0);
        assert_eq!(builder.window_bucket_in_bits, 0);
        builder.build(
            |c| async move {
                assert!(c.is_liveliness_in(&[GraphLivelinessState::Building]));
                Ok(())
            },
            ScheduleAs::SoloAct,
        );
    }

    #[test]
    fn test_work_new() {
        let work = Work::new(50.0).expect("internal error");
        assert_eq!(work.work, 5000);
    }

    #[test]
    fn test_mcpu_new() {
        let mcpu = MCPU::new(512).expect("internal error");
        assert_eq!(mcpu.mcpu, 512);
    }

    #[test]
    fn test_percentile_new() {
        let percentile = Percentile::new(99.0).expect("internal error");
        assert_eq!(percentile.percentile(), 99.0);
    }
}
