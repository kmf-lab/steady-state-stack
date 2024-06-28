//! The `actor_builder` module provides structures and functions to create, configure, and manage actors within a system.
//! This module includes the `ActorBuilder` for building actors, `ActorTeam` for managing groups of actors, and various utility
//! functions and types to support actor creation and telemetry monitoring.

use std::any::Any;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, AtomicUsize};
use std::time::{Duration, Instant};
use core::default::Default;
use std::collections::VecDeque;

use futures::channel::oneshot;
use futures::channel::oneshot::{Receiver, Sender};
use futures_util::lock::Mutex;
use log::*;
use futures_util::future::{BoxFuture, select_all};

use crate::{abstract_executor, AlertColor, Graph, Metric, StdDev, SteadyContext, Trigger};
use crate::graph_liveliness::{ActorIdentity, GraphLiveliness};
use crate::graph_testing::{SideChannel, SideChannelHub};
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;

use std::panic::{catch_unwind, AssertUnwindSafe};

/// The `ActorBuilder` struct is responsible for building and configuring actors.
/// It contains various settings related to telemetry, triggers, and actor identification.
#[derive(Clone)]
pub struct ActorBuilder {
    name: &'static str,
    suffix: Option<usize>,
    args: Arc<Box<dyn Any + Send + Sync>>,
    telemetry_tx: Arc<RwLock<Vec<CollectorDetail>>>,
    channel_count: Arc<AtomicUsize>,
    runtime_state: Arc<RwLock<GraphLiveliness>>,
    monitor_count: Arc<AtomicUsize>,

    refresh_rate_in_bits: u8,
    window_bucket_in_bits: u8,
    usage_review: bool,

    percentiles_mcpu: Vec<Percentile>,
    percentiles_work: Vec<Percentile>,
    std_dev_mcpu: Vec<StdDev>,
    std_dev_work: Vec<StdDev>,
    trigger_mcpu: Vec<(Trigger<MCPU>, AlertColor)>,
    trigger_work: Vec<(Trigger<Work>, AlertColor)>,
    avg_mcpu: bool,
    avg_work: bool,
    frame_rate_ms: u64,
    oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    oneshot_startup_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    backplane: Arc<Mutex<Option<SideChannelHub>>>,
}

type FutureBuilderType = Box<dyn FnMut() -> (BoxFuture<'static, Result<(), Box<dyn Error>>>, bool) + Send>;

/// The `ActorTeam` struct manages a collection of actors, facilitating their coordinated execution.
#[derive(Default)]
pub struct ActorTeam {
    future_builder: VecDeque<FutureBuilderType>,
}

impl ActorTeam {
    /// Creates a new instance of `ActorTeam`.
    pub fn new() -> Self {
        ActorTeam {
            future_builder: VecDeque::new(),
        }
    }

    /// Adds an actor to the team with the specified context and frame rate.
    fn add_actor<F, I>(&mut self, mut actor_startup_receiver: Option<Receiver<()>>, context_archetype: SteadyContextArchetype<I>, frame_rate_ms: u64)
        where
            F: 'static + Future<Output = Result<(), Box<dyn Error>>> + Send,
            I: 'static + Fn(SteadyContext) -> F + Send + Sync,
    {
        self.future_builder.push_back({
            let context_archetype = context_archetype.clone();
            Box::new(move || {
                let boxed_future: BoxFuture<'static, Result<(), Box<dyn Error>>> =
                    Box::pin(build_actor_future(&mut actor_startup_receiver, context_archetype.clone(), frame_rate_ms));
                (boxed_future, false)
            }) as Box<dyn FnMut() -> (BoxFuture<'static, Result<(), Box<dyn Error>>>, bool) + Send>
        });
    }

    /// Transfers the front actor to another `ActorTeam`.
    pub fn transfer_front_to(&mut self, other: &mut Self) -> bool {
        if let Some(f) = self.future_builder.pop_front() {
            other.future_builder.push_back(f);
            true
        } else {
            false
        }
    }

    /// Transfers the back actor to another `ActorTeam`.
    pub fn transfer_back_to(&mut self, other: &mut Self) -> bool {
        if let Some(f) = self.future_builder.pop_back() {
            other.future_builder.push_back(f);
            true
        } else {
            false
        }
    }

    /// Spawns all actors in the team, returning the count of actors spawned.
    pub fn spawn(mut self) -> usize {
        if self.future_builder.is_empty() {
            return 0; // Nothing to spawn, so return
        }

        let count = self.future_builder.len();
        let super_task = {
            async move {
                let mut actor_future_vec = Vec::with_capacity(count);

                for f in &mut self.future_builder {
                    let (future, _drive_io) = f();
                    actor_future_vec.push(future);
                }

                loop {
                    // This result is for panics.
                    // Panic will cause all the grouped actors under this thread to be restarted together.
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        let (result, index, _remaining_futures) = launch_actor(select_all(&mut actor_future_vec));

                        if let Err(e) = result {
                            error!("Actor {:?} error {:?}", index, e); // TODO: need ident for index.
                            // Rebuild the single actor which failed
                            actor_future_vec[index] = self.future_builder[index]().0;
                            true
                        } else {
                            // This actor was finished, so remove it from the list
                            drop(actor_future_vec.remove(index));
                            !actor_future_vec.is_empty() // true we keep running
                        }
                    }));
                    match result {
                        Ok(true) => {
                            // Keep running the loop
                        }
                        Ok(false) => {
                            // trace!("Actor {:?} finished ", name);
                            break; // Exit the loop, we are all done
                        }
                        Err(e) => {
                            // Actor Panic
                            error!("Actor panic {:?}", e);
                            actor_future_vec.clear();
                            // We must rebuild all actors since we do not know what happened to the thread
                            // EVEN those finished will be restarted.
                            for f in &mut self.future_builder {
                                let (future, _drive_io) = f();
                                actor_future_vec.push(future);
                            }
                            // Continue running
                        }
                    }
                }
            }
        };
        abstract_executor::spawn_detached(super_task);
        count
    }
}

/// WARNING: do not rename this function without change of backtrace printing since we use this as a "stop" to shorten traces.
pub fn launch_actor<F: Future<Output = T>, T>(future: F) -> T {
    nuclei::block_on(future)
}

/// The `SteadyContextArchetype` struct serves as a template for building actor contexts,
/// encapsulating all the necessary parameters and state.
struct SteadyContextArchetype<I> {
    build_actor_exec: Arc<I>,
    runtime_state: Arc<RwLock<GraphLiveliness>>,
    channel_count: Arc<AtomicUsize>,
    ident: ActorIdentity,
    args: Arc<Box<dyn Any + Send + Sync>>,
    all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    actor_metadata: Arc<ActorMetaData>,
    oneshot_shutdown_vec: Arc<Mutex<Vec<Sender<()>>>>,
    oneshot_shutdown: Arc<Mutex<Receiver<()>>>,
    node_tx_rx: Option<Arc<Mutex<SideChannel>>>,
    instance_id: Arc<AtomicU32>,
}

impl<I> Clone for SteadyContextArchetype<I> {
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
            instance_id: self.instance_id.clone(),
        }
    }
}

impl ActorBuilder {
    /// Creates a new `ActorBuilder` instance, initializing it with defaults and configurations derived from the given `Graph`.
    ///
    /// # Arguments
    ///
    /// * `graph` - A mutable reference to the `Graph` from which to inherit settings.
    ///
    /// # Returns
    ///
    /// A new instance of `ActorBuilder`.
    pub fn new(graph: &mut Graph) -> ActorBuilder {
        ActorBuilder {
            backplane: graph.backplane.clone(),
            name: "",
            suffix: None,
            monitor_count: graph.monitor_count.clone(),
            args: graph.args.clone(),
            telemetry_tx: graph.all_telemetry_rx.clone(),
            channel_count: graph.channel_count.clone(),
            runtime_state: graph.runtime_state.clone(),
            refresh_rate_in_bits: 6, // 1 << 6 == 64
            window_bucket_in_bits: 5, // 1 << 5 == 32
            oneshot_shutdown_vec: graph.oneshot_shutdown_vec.clone(),
            oneshot_startup_vec: graph.oneshot_startup_vec.clone(),
            percentiles_mcpu: Vec::with_capacity(0),
            percentiles_work: Vec::with_capacity(0),
            std_dev_mcpu: Vec::with_capacity(0),
            std_dev_work: Vec::with_capacity(0),
            trigger_mcpu: Vec::with_capacity(0),
            trigger_work: Vec::with_capacity(0),
            avg_mcpu: false,
            avg_work: false,
            frame_rate_ms: graph.telemetry_production_rate_ms,
            usage_review: false,
        }
    }

    /// Sets the compute refresh window floor and bucket size for telemetry, adjusting the resolution of performance metrics.
    ///
    /// # Arguments
    ///
    /// * `refresh` - The minimum refresh rate as a `Duration`.
    /// * `window` - The size of the window as a `Duration`.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified compute refresh window configuration.
    pub fn with_compute_refresh_window_floor(&self, refresh: Duration, window: Duration) -> Self {
        let result = self.clone();
        // We must compute the refresh rate first before we do the window
        let frames_per_refresh = refresh.as_micros() / (1000u128 * self.frame_rate_ms as u128);
        let refresh_in_bits = (frames_per_refresh as f32).log2().ceil() as u8;
        let refresh_in_micros = (1000u128 << refresh_in_bits) * self.frame_rate_ms as u128;
        // Now compute the window based on our new bucket size
        let buckets_per_window: f32 = window.as_micros() as f32 / refresh_in_micros as f32;
        // Find the next largest power of 2
        let window_in_bits = buckets_per_window.log2().ceil() as u8;
        result.with_compute_refresh_window_bucket_bits(refresh_in_bits, window_in_bits)
    }

    /// Directly sets the compute refresh window bucket sizes in bits, providing fine-grained control over telemetry resolution.
    ///
    /// # Arguments
    ///
    /// * `refresh_bucket_in_bits` - The refresh rate in bits.
    /// * `window_bucket_in_bits` - The window size in bits.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified bucket sizes.
    pub fn with_compute_refresh_window_bucket_bits(&self, refresh_bucket_in_bits: u8, window_bucket_in_bits: u8) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = refresh_bucket_in_bits;
        result.window_bucket_in_bits = window_bucket_in_bits;
        result
    }

    /// Assigns a name to the actor, used for identification and possibly logging.
    ///
    /// # Arguments
    ///
    /// * `name` - A static string slice representing the actor's name.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified name.
    pub fn with_name(&self, name: &'static str) -> Self {
        let mut result = self.clone();
        result.name = name;
        result
    }

    /// Appends a suffix to the actor's name, aiding in the differentiation of actor instances.
    ///
    /// # Arguments
    ///
    /// * `suffix` - A unique identifier to be appended to the actor's name.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the name suffix applied.
    pub fn with_name_suffix(&self, suffix: usize) -> Self {
        let mut result = self.clone();
        result.suffix = Some(suffix);
        result
    }

    /// Sets both the name and suffix for the actor.
    ///
    /// # Arguments
    ///
    /// * `name` - A static string slice representing the actor's name.
    /// * `suffix` - A unique identifier to be appended to the actor's name.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the name and suffix applied.
    pub fn with_name_and_suffix(&self, name: &'static str, suffix: usize) -> Self {
        let mut result = self.clone();
        result.name = name;
        result.suffix = Some(suffix);
        result
    }

    /// Configures the actor to monitor a specific CPU usage percentile for performance analysis.
    ///
    /// # Arguments
    ///
    /// * `config` - The `Percentile` to monitor for CPU usage.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified CPU usage percentile configuration.
    pub fn with_mcpu_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_mcpu.push(config);
        result
    }

    /// Configures the actor to monitor a specific workload percentile, aiding in workload analysis and optimization.
    ///
    /// # Arguments
    ///
    /// * `config` - The `Percentile` to monitor for workload performance.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified workload percentile configuration.
    pub fn with_work_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_work.push(config);
        result
    }

    /// Enables average CPU usage monitoring for the actor, smoothing out short-term fluctuations in usage metrics.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with average CPU monitoring enabled.
    pub fn with_avg_mcpu(&self) -> Self {
        let mut result = self.clone();
        result.avg_mcpu = true;
        result
    }

    /// Enables average workload monitoring, providing a more consistent view of the actor's workload over time.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with average workload monitoring enabled.
    pub fn with_avg_work(&self) -> Self {
        let mut result = self.clone();
        result.avg_work = true;
        result
    }

    /// Sets a CPU usage threshold that, when exceeded, triggers an alert, helping maintain system performance and stability.
    ///
    /// # Arguments
    ///
    /// * `bound` - The trigger condition based on CPU usage.
    /// * `color` - The `AlertColor` to be used when the condition is met.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified CPU trigger condition.
    pub fn with_mcpu_trigger(&self, bound: Trigger<MCPU>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_mcpu.push((bound, color));
        result
    }

    /// Sets a workload threshold that, when exceeded, triggers an alert, assisting in proactive system monitoring.
    ///
    /// # Arguments
    ///
    /// * `bound` - The trigger condition based on workload.
    /// * `color` - The `AlertColor` to be used when the condition is met.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified workload trigger condition.
    pub fn with_work_trigger(&self, bound: Trigger<Work>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_work.push((bound, color));
        result
    }

    /// Sets the floor for compute window size, ensuring performance metrics are based on a sufficiently large sample.
    ///
    /// # Arguments
    ///
    /// * `duration` - The minimum duration for the compute window.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified compute window floor.
    pub fn with_compute_window_floor(&self, duration: Duration) -> Self {
        let result = self.clone();
        let millis: u128 = duration.as_micros();
        let frame_ms: u128 = 1000u128 * self.frame_rate_ms as u128;
        let est_buckets = millis / frame_ms;
        // Find the next largest power of 2
        let window_bucket_in_bits = (est_buckets as f32).log2().ceil() as u8;
        result.with_compute_window_bucket_bits(window_bucket_in_bits)
    }

    /// Directly sets the compute window size in bits, offering precise control over the performance metrics' temporal resolution.
    ///
    /// # Arguments
    ///
    /// * `window_bucket_in_bits` - The size of the window in bits.
    ///
    /// # Note
    ///
    /// The default window size is 1 second.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified window size.
    pub fn with_compute_window_bucket_bits(&self, window_bucket_in_bits: u8) -> Self {
        assert!(window_bucket_in_bits <= 30);
        let mut result = self.clone();
        result.window_bucket_in_bits = window_bucket_in_bits;
        result
    }

    /// Completes the actor configuration and initiates its execution with the provided logic.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic, a function taking a `SteadyContext` and returning `F`.
    ///
    /// # Arguments
    ///
    /// * `exec` - The execution logic for the actor.
    pub fn build_spawn<F, I>(self, build_actor_exec: I)
        where
            I: Fn(SteadyContext) -> F + 'static + Sync + Send,
            F: Future<Output = Result<(), Box<dyn Error>>> + Send + 'static,
    {
        let rate_ms = self.frame_rate_ms;
        let mut actor_startup_receiver = Some(Self::build_startup_oneshot(self.oneshot_startup_vec.clone()));
        let context_archetype = self.single_actor_exec_archetype(build_actor_exec);

        abstract_executor::spawn_detached(Box::pin(async move {
            loop {
                let future = build_actor_future(&mut actor_startup_receiver, context_archetype.clone(), rate_ms);

                let result = catch_unwind(AssertUnwindSafe(|| launch_actor(future)));

                match result {
                    Ok(_) => {
                        // trace!("Actor {:?} finished ", name);
                        break; // Exit the loop we are all done
                    }
                    Err(e) => {
                        if let Some(specific_error) = e.downcast_ref::<std::io::Error>() {
                            warn!("IO Error encountered: {}", specific_error);
                        } else if let Some(specific_error) = e.downcast_ref::<String>() {
                            warn!("String Error encountered: {}", specific_error);
                        }
                        // Panic or an actor Error, log and continue in the loop
                        warn!("Restarting: {:?} ", context_archetype.ident);
                    }
                }
            }
        }));
    }

    /// Adds an actor to the specified `ActorTeam`, enabling group execution.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic, a function taking a `SteadyContext` and returning `F`.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - The execution logic for the actor.
    /// * `target` - The `ActorTeam` to which the actor will be added.
    pub fn build_join<F, I>(self, build_actor_exec: I, target: &mut ActorTeam)
        where
            I: Fn(SteadyContext) -> F + 'static + Sync + Send,
            F: Future<Output = Result<(), Box<dyn Error>>> + 'static + Send,
    {
        let rate = self.frame_rate_ms;
        let actor_startup_receiver = Some(Self::build_startup_oneshot(self.oneshot_startup_vec.clone()));
        target.add_actor(actor_startup_receiver, self.single_actor_exec_archetype(build_actor_exec), rate);
    }

    /// Creates a `SteadyContextArchetype` for actor execution, encapsulating the necessary parameters and state.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The future returned by the execution logic.
    /// * `I` - The execution logic, a function taking a `SteadyContext` and returning `F`.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - The execution logic for the actor.
    ///
    /// # Returns
    ///
    /// A `SteadyContextArchetype` instance configured with the actor's execution logic.
    fn single_actor_exec_archetype<F, I>(self, build_actor_exec: I) -> SteadyContextArchetype<I>
        where
            I: Fn(SteadyContext) -> F + 'static + Sync + Send,
            F: Future<Output = Result<(), Box<dyn Error>>> + Send + 'static,
    {
        let name = self.name;
        let telemetry_tx = self.telemetry_tx.clone();
        let channel_count = self.channel_count.clone();
        let runtime_state = self.runtime_state.clone();
        let args = self.args.clone();
        let oneshot_shutdown_vec = self.oneshot_shutdown_vec.clone();
        let backplane = self.backplane.clone();

        let id = self.monitor_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let immutable_identity = ActorIdentity { id, name };
        // info!(" Actor {:?} defined ", immutable_identity);
        let immutable_actor_metadata = self.build_actor_metadata(id, name).clone();

        /////////////////////////////////////////////
        // This is used only when run under testing
        ////////////////////////////////////////////
        let immutable_node_tx_rx = abstract_executor::block_on(async move {
            let mut backplane = backplane.lock().await;
            // If the backplane is enabled then register every node name for use
            if let Some(pb) = &mut *backplane {
                let max_channel_capacity = 8;
                // TODO: name may not be good enough here and may need prefix as well.
                pb.register_node(name, max_channel_capacity);
                pb.node_tx_rx(name)
            } else {
                None
            }
        });

        ////////////////////////////////////////////
        // Before starting the actor setup our shutdown oneshot
        ////////////////////////////////////////////
        // This single one shot is kept no matter how many times actor is restarted
        let immutable_oneshot_shutdown = {
            let (send_shutdown_notice_to_periodic_wait, oneshot_shutdown) = oneshot::channel();
            let oneshot_shutdown_vec = oneshot_shutdown_vec.clone();
            abstract_executor::block_on(async move {
                let mut v = oneshot_shutdown_vec.lock().await;
                v.push(send_shutdown_notice_to_periodic_wait);
            });
            Arc::new(Mutex::new(oneshot_shutdown))
        };

        let restart_counter = Arc::new(AtomicU32::new(0));

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
            build_actor_exec: Arc::new(build_actor_exec),
            instance_id: restart_counter,
        }
    }

    /// Builds a oneshot channel for actor startup, used to synchronize actor initialization.
    ///
    /// # Arguments
    ///
    /// * `oneshot_startup_vec` - A shared vector of startup channels.
    ///
    /// # Returns
    ///
    /// A receiver for the startup oneshot channel.
    fn build_startup_oneshot(oneshot_startup_vec: Arc<Mutex<Vec<Sender<()>>>>) -> Receiver<()> {
        let (actor_startup_sender, actor_startup_receiver) = oneshot::channel();
        // We do not normally expect this to not get the lock
        if let Some(mut v) = oneshot_startup_vec.clone().try_lock() {
            // Fast path
            v.push(actor_startup_sender);
        } else {
            abstract_executor::block_on(async move {
                oneshot_startup_vec.clone().lock().await.push(actor_startup_sender);
            });
        }
        actor_startup_receiver
    }

    /// Constructs actor metadata for the given actor ID and name, encapsulating telemetry and trigger configurations.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for the actor.
    /// * `name` - The name of the actor.
    ///
    /// # Returns
    ///
    /// An `Arc` containing the actor metadata.
    fn build_actor_metadata(&self, id: usize, name: &'static str) -> Arc<ActorMetaData> {
        Arc::new(ActorMetaData {
            id,
            name,
            avg_mcpu: self.avg_mcpu,
            avg_work: self.avg_work,
            percentiles_mcpu: self.percentiles_mcpu.clone(),
            percentiles_work: self.percentiles_work.clone(),
            std_dev_mcpu: self.std_dev_mcpu.clone(),
            std_dev_work: self.std_dev_work.clone(),
            trigger_mcpu: self.trigger_mcpu.clone(),
            trigger_work: self.trigger_work.clone(),
            usage_review: self.usage_review,
            refresh_rate_in_bits: self.refresh_rate_in_bits,
            window_bucket_in_bits: self.window_bucket_in_bits,
        })
    }
}

/// Constructs a future for the actor, incorporating startup synchronization and context initialization.
///
/// # Type Parameters
///
/// * `F` - The future returned by the execution logic.
/// * `I` - The execution logic, a function taking a `SteadyContext` and returning `F`.
///
/// # Arguments
///
/// * `actor_startup_receiver` - An optional receiver for the startup synchronization channel.
/// * `builder_source` - The archetype for building the actor context.
/// * `frame_rate_ms` - The frame rate in milliseconds.
///
/// # Returns
///
/// A future representing the actor's execution.
fn build_actor_future<F, I>(
    actor_startup_receiver: &mut Option<Receiver<()>>,
    builder_source: SteadyContextArchetype<I>,
    frame_rate_ms: u64,
) -> F
    where
        I: Fn(SteadyContext) -> F + 'static + Sync + Send,
        F: Future<Output = Result<(), Box<dyn Error>>> + Send + 'static,
{
    if let Some(actor_startup_receiver_internal) = actor_startup_receiver.take() {
        // No start signal yet so we block until we get it
        let _ = abstract_executor::block_on(actor_startup_receiver_internal);
    }
    (builder_source.build_actor_exec)(SteadyContext {
        runtime_state: builder_source.runtime_state,
        channel_count: builder_source.channel_count,
        ident: builder_source.ident,
        args: builder_source.args,
        all_telemetry_rx: builder_source.all_telemetry_rx,
        actor_metadata: builder_source.actor_metadata,
        oneshot_shutdown_vec: builder_source.oneshot_shutdown_vec,
        oneshot_shutdown: builder_source.oneshot_shutdown,
        node_tx_rx: builder_source.node_tx_rx,
        instance_id: builder_source.instance_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        last_periodic_wait: Default::default(),
        is_in_graph: true,
        actor_start_time: Instant::now(),
        frame_rate_ms,
    })
}

/// Implements the `Metric` trait for the `Work` struct, enabling it to be used as a telemetry metric.
impl Metric for Work {}

/// The `Work` struct represents a unit of work, used for workload analysis and monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    work: u16, // out of 10000 where 10000 is 100%
}

impl Work {
    /// Creates a new `Work` instance with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value of the work, as a percentage.
    ///
    /// # Returns
    ///
    /// An `Option` containing the `Work` instance if the value is valid.
    pub fn new(value: f32) -> Option<Self> {
        if (0.0..=100.00).contains(&value) {
            Some(Work { work: (value * 100.0) as u16 }) // 10_000 is 100%
        } else {
            None
        }
    }

    /// Returns the rational representation of the work.
    pub fn rational(&self) -> (u64, u64) {
        (self.work as u64, 10_000)
    }

    /// Returns a `Work` instance representing 10% work.
    pub fn p10() -> Self {
        Work { work: 1000 }
    }

    /// Returns a `Work` instance representing 20% work.
    pub fn p20() -> Self {
        Work { work: 2000 }
    }

    /// Returns a `Work` instance representing 30% work.
    pub fn p30() -> Self {
        Work { work: 3000 }
    }

    /// Returns a `Work` instance representing 40% work.
    pub fn p40() -> Self {
        Work { work: 4000 }
    }

    /// Returns a `Work` instance representing 50% work.
    pub fn p50() -> Self {
        Work { work: 5000 }
    }

    /// Returns a `Work` instance representing 60% work.
    pub fn p60() -> Self {
        Work { work: 6000 }
    }

    /// Returns a `Work` instance representing 70% work.
    pub fn p70() -> Self {
        Work { work: 7000 }
    }

    /// Returns a `Work` instance representing 80% work.
    pub fn p80() -> Self {
        Work { work: 8000 }
    }

    /// Returns a `Work` instance representing 90% work.
    pub fn p90() -> Self {
        Work { work: 9000 }
    }

    /// Returns a `Work` instance representing 100% work.
    pub fn p100() -> Self {
        Work { work: 10_000 }
    }
}

/// Implements the `Metric` trait for the `MCPU` struct, enabling it to be used as a telemetry metric.
impl Metric for MCPU {}

/// The `MCPU` struct represents a unit of CPU usage, used for performance analysis and monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MCPU {
    mcpu: u16, // max 1024
}

impl MCPU {
    /// Creates a new `MCPU` instance with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value of the MCPU, up to a maximum of 1024.
    ///
    /// # Returns
    ///
    /// An `Option` containing the `MCPU` instance if the value is valid.
    pub fn new(value: u16) -> Option<Self> {
        if value <= 1024 {
            Some(Self { mcpu: value })
        } else {
            None
        }
    }

    /// Returns the rational representation of the MCPU.
    pub fn rational(&self) -> (u64, u64) {
        (self.mcpu as u64, 1024)
    }

    /// Returns an `MCPU` instance representing 16 MCPU.
    pub fn m16() -> Self {
        MCPU { mcpu: 16 }
    }

    /// Returns an `MCPU` instance representing 64 MCPU.
    pub fn m64() -> Self {
        MCPU { mcpu: 64 }
    }

    /// Returns an `MCPU` instance representing 256 MCPU.
    pub fn m256() -> Self {
        MCPU { mcpu: 256 }
    }

    /// Returns an `MCPU` instance representing 512 MCPU.
    pub fn m512() -> Self {
        MCPU { mcpu: 512 }
    }

    /// Returns an `MCPU` instance representing 768 MCPU.
    pub fn m768() -> Self {
        MCPU { mcpu: 768 }
    }

    /// Returns an `MCPU` instance representing 1024 MCPU.
    pub fn m1024() -> Self {
        MCPU { mcpu: 1024 }
    }
}

/// The `Percentile` struct represents a percentile value, used for performance and workload analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Percentile(f64);

impl Percentile {
    /// Creates a new `Percentile` instance with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value of the percentile, between 0.0 and 100.0.
    ///
    /// # Returns
    ///
    /// An `Option` containing the `Percentile` instance if the value is valid.
    fn new(value: f64) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Returns a `Percentile` instance representing the 25th percentile.
    pub fn p25() -> Self {
        Self(25.0)
    }

    /// Returns a `Percentile` instance representing the 50th percentile.
    pub fn p50() -> Self {
        Self(50.0)
    }

    /// Returns a `Percentile` instance representing the 75th percentile.
    pub fn p75() -> Self {
        Self(75.0)
    }

    /// Returns a `Percentile` instance representing the 90th percentile.
    pub fn p90() -> Self {
        Self(90.0)
    }

    /// Returns a `Percentile` instance representing the 80th percentile.
    pub fn p80() -> Self {
        Self(80.0)
    }

    /// Returns a `Percentile` instance representing the 96th percentile.
    pub fn p96() -> Self {
        Self(96.0)
    }

    /// Returns a `Percentile` instance representing the 99th percentile.
    pub fn p99() -> Self {
        Self(99.0)
    }

    /// Allows custom values within the valid range.
    ///
    /// # Arguments
    ///
    /// * `value` - The custom percentile value.
    ///
    /// # Returns
    ///
    /// An `Option` containing the `Percentile` instance if the value is valid.
    pub fn custom(value: f64) -> Option<Self> {
        Self::new(value)
    }

    /// Getter to access the inner percentile value.
    pub fn percentile(&self) -> f64 {
        self.0
    }
}


