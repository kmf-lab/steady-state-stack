// ss[related actor.regeneration-survives]
use super::affinity::CoreBalancer;
use super::context::{DynCall, NonSendWrapper, SteadyContextArchetype};
use crate::dot::RemoteDetails;
use crate::graph_liveliness::{ActorIdentity, GraphLiveliness};
use crate::graph_testing::StageManager;
use crate::monitor::ActorMetaData;
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry_window::compute_refresh_window_frames;
use crate::*;
use aeron::aeron::Aeron;
use async_lock::Barrier;
use futures::channel::oneshot::{Receiver, Sender};
use futures_util::future::Shared;
use futures_util::FutureExt;
use futures_util::lock::{Mutex, MutexGuard};
use parking_lot::RwLock;
use std::any::Any;
use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// THE `ActorBuilder` struct is responsible for constructing and configuring actors within the system.
/// It provides a fluent interface to set various properties and behaviors of the actor, such as telemetry settings,
/// trigger conditions, and execution parameters. Once configured, the builder can spawn the actor either standalone
/// or as part of a `Troupe`.
#[derive(Clone)]
// ss[related actor.regeneration-survives]
pub struct ActorBuilder {
    /// THE name of the actor, used for identification in telemetry and logging.
    pub(crate) actor_name: ActorName,
    /// Shared arguments passed to the actor, accessible via the `args` method in `SteadyContext`.
    pub(crate) args: Arc<Box<dyn Any + Send + Sync>>,
    /// Telemetry transmitter for collecting and sending actor metrics.
    pub(crate) telemetry_tx: Arc<RwLock<Vec<CollectorDetail>>>,
    /// Shared counter for the number of channels in the graph.
    pub(crate) channel_count: Arc<AtomicUsize>,
    /// Shared liveliness state of the graph, used for managing actor lifecycle.
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    /// Shared counter for the number of actors in the graph.
    pub(crate) actor_count: Arc<AtomicUsize>,
    /// Mutex for synchronizing thread operations, particularly for core affinity settings.
    pub(crate) thread_lock: Arc<Mutex<()>>,
    /// List of CPU cores to exclude from actor assignment.
    pub(crate) excluded_cores: Vec<usize>,
    /// Optional core balancer for distributing actors across available cores.
    pub(crate) core_balancer: Option<CoreBalancer>,
    /// Optional explicit core assignment for the actor.
    pub(crate) explicit_core: Option<usize>,
    /// Bit shift value determining the refresh rate for telemetry data.
    pub(crate) refresh_rate_in_bits: u8,
    /// Bit shift value determining the window bucket size for metrics aggregation.
    pub(crate) window_bucket_in_bits: u8,
    /// Flag indicating whether usage review is enabled for the actor.
    pub(crate) usage_review: bool,
    /// Percentiles to monitor for CPU usage metrics.
    pub(crate) percentiles_mcpu: Vec<Percentile>,
    /// Percentiles to monitor for workload metrics.
    pub(crate) percentiles_load: Vec<Percentile>,
    /// Standard deviations to monitor for CPU usage metrics.
    pub(crate) std_dev_mcpu: Vec<StdDev>,
    /// Standard deviations to monitor for workload metrics.
    pub(crate) std_dev_load: Vec<StdDev>,
    /// Triggers for CPU usage that raise alerts with associated colors.
    pub(crate) trigger_mcpu: Vec<(Trigger<MCPU>, AlertColor)>,
    /// Triggers for workload that raise alerts with associated colors.
    pub(crate) trigger_load: Vec<(Trigger<Work>, AlertColor)>,
    /// Flag indicating whether to include thread information in telemetry data.
    pub(crate) show_thread_info: bool,
    /// Flag indicating whether to monitor average CPU usage.
    pub(crate) avg_mcpu: bool,
    /// Flag indicating whether to monitor average workload.
    pub(crate) avg_load: bool,
    /// Frame rate in milliseconds for telemetry data collection.
    pub(crate) frame_rate_ms: u64,
    /// Shared vector of oneshot senders for shutdown notifications.
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    /// Backplane for side-channel communications, primarily used in testing.
    pub(crate) backplane: Arc<Mutex<Option<StageManager>>>,
    /// Shared counter for the number of actor teams.
    pub(crate) team_count: Arc<AtomicUsize>,
    /// Optional details for remote communication in distributed systems.
    pub(crate) remote_details: Option<RemoteDetails>,
    /// Flag indicating whether to prevent simulation, ensuring real execution.
    pub(crate) never_simulate: bool,
    /// Lazily initialized Aeron media driver for communication.
    pub(crate) aeron_meda_driver: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    /// Optional barrier for synchronizing actor shutdown.
    pub shutdown_barrier: Option<Arc<Barrier>>,
    /// Flag indicating whether the actor is for testing purposes.
    pub(crate) is_for_test: bool,
    /// Optional stack size for the actor.
    pub(crate) stack_size: Option<usize>,
    /// Universal list of all actors in the graph.
    pub(crate) actor_catalog: Arc<RwLock<Vec<ActorIdentity>>>,
    /// Actor base names that run real `internal_behavior` in test graphs (pipeline processors).
    pub(crate) test_pipeline_internal_names: Arc<HashSet<&'static str>>,
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
    // ss[related actor.regeneration-survives]
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
            aeron_meda_driver: graph.aeron.clone(),
            shutdown_barrier: graph.shutdown_barrier.clone(),
            is_for_test: graph.is_for_testing,
            stack_size: graph.default_stack_size,
            actor_catalog: graph.actor_catalog.clone(),
            test_pipeline_internal_names: graph.test_pipeline_internal_names.clone(),
        }
    }

    /// Sets the compute refresh window floor and bucket size for telemetry, adjusting the resolution of performance metrics.
    ///
    /// This method fine-tunes telemetry data collection by specifying the minimum refresh rate and window size for
    /// metrics aggregation.
    ///
    /// **Effective window vs. wall clock:** [`crate::telemetry_window::compute_refresh_window_frames`] rounds bucket
    /// counts up to powers of two. The displayed “Window” span is approximately
    /// `telemetry_frame_ms × 2^(refresh_bits + window_bits)`, so a `(1s, 10s)` floor with a ~100ms collector frame
    /// often yields **~12.8s** of samples, not exactly 10s. Use a shorter `window` argument if you need **Avg mCPU**
    /// to converge faster; defaults come from [`ActorBuilder::new`] (`1s` / `10s`).
    ///
    /// # Arguments
    ///
    /// * `refresh` - THE minimum refresh rate as a `Duration`.
    /// * `window` - THE size of the window as a `Duration`.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the updated compute refresh window configuration.
    // ss[related actor.regeneration-survives]
    pub fn with_compute_refresh_window_floor(&self, refresh: Duration, window: Duration) -> Self {
        let mut result = self.clone();
        let (refresh_in_bits, window_in_bits) = ActorBuilder::internal_compute_refresh_window(
            self.frame_rate_ms as u128,
            refresh,
            window,
        );
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
    // ss[related actor.regeneration-survives]
    pub fn with_core_exclusion(&self, cores: Vec<usize>) -> Self {
        let mut result = self.clone();
        result.excluded_cores = cores;
        result
    }

    /// Configures the actor to use a core balancer for dynamic core allocation.
    ///
    /// THE core balancer distributes actors across available cores to optimize resource usage.
    ///
    /// # Arguments
    ///
    /// * `balancer` - An instance of `CoreBalancer` for core allocation.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified core balancer.
    // ss[related actor.regeneration-survives]
    pub fn with_core_balancing(&self, balancer: CoreBalancer) -> Self {
        let mut result = self.clone();
        result.core_balancer = Some(balancer);
        result
    }

    /// Assigns the actor to a specific CPU core explicitly, overriding any balancing or default assignment.
    ///
    /// # Arguments
    ///
    /// * `one_offset_core` - THE one-based index of the core to assign the actor to.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the explicit core assignment.
    // ss[related actor.regeneration-survives]
    pub fn with_explicit_core(&self, one_offset_core: u16) -> Self {
        let mut result = self.clone();
        assert!(
            one_offset_core > 0,
            "Core index must be greater than zero and match your OS task manager."
        );
        let zero_offset_core = one_offset_core - 1;
        result.explicit_core = Some(zero_offset_core.into());
        result
    }

    /// Disables telemetry metric collection for the actor, useful for performance-critical scenarios.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with telemetry disabled.
    // ss[related actor.regeneration-survives]
    pub fn with_no_refresh_window(&self) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = 0;
        result.window_bucket_in_bits = 0;
        result
    }

    /// Computes the refresh rate and window bucket size in bits based on frame rate and durations.
    ///
    /// Delegates to [`crate::telemetry_window::compute_refresh_window_frames`]: one sample per
    /// telemetry frame (same cadence as channel edge rollups).
    ///
    /// # Arguments
    ///
    /// * `frame_rate_ms` - THE frame rate in milliseconds.
    /// * `refresh` - THE desired refresh duration.
    /// * `window` - THE desired window duration.
    ///
    /// # Returns
    ///
    /// A tuple of `(refresh_in_bits, window_in_bits)` representing the computed values.
    // ss[related actor.regeneration-survives]
    pub(crate) fn internal_compute_refresh_window(
        frame_rate_ms: u128,
        refresh: Duration,
        window: Duration,
    ) -> (u8, u8) {
        compute_refresh_window_frames(frame_rate_ms, refresh, window)
    }

    /// Configures the actor to monitor a specific CPU usage percentile for performance analysis.
    ///
    /// # Arguments
    ///
    /// * `config` - THE `Percentile` to monitor for CPU usage.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified CPU usage percentile.
    // ss[related actor.regeneration-survives]
    pub fn with_mcpu_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_mcpu.push(config);
        result
    }

    /// Sets the actor's name with a suffix for telemetry identification.
    ///
    /// # Arguments
    ///
    /// * `name` - THE base name of the actor.
    /// * `suffix` - A numeric suffix for uniqueness.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified name and suffix.
    // ss[related actor.regeneration-survives]
    pub fn with_name_and_suffix(&self, name: &'static str, suffix: usize) -> Self {
        let mut result = self.clone();
        result.actor_name = ActorName::new(name, Some(suffix));
        result
    }

    /// Sets the actor's name for telemetry identification.
    ///
    /// # Arguments
    ///
    /// * `name` - THE name of the actor.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified name.
    // ss[related actor.regeneration-survives]
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
    // ss[related actor.regeneration-survives]
    pub fn never_simulate(&self, never_simulate: bool) -> Self {
        let mut result = self.clone();
        result.never_simulate = never_simulate;
        result
    }

    /// Configures the actor to monitor a specific workload percentile for performance analysis.
    ///
    /// # Arguments
    ///
    /// * `config` - THE `Percentile` to monitor for workload.
    ///
    /// # Returns
    ///
    /// a new `ActorBuilder` instance with the specified workload percentile.
    // ss[related actor.regeneration-survives]
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
    // ss[related actor.regeneration-survives]
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
    // ss[related actor.regeneration-survives]
    pub fn with_load_avg(&self) -> Self {
        let mut result = self.clone();
        result.avg_load = true;
        result
    }

    /// Sets a CPU usage trigger that raises an alert when exceeded.
    ///
    /// # Arguments
    ///
    /// * `bound` - THE trigger condition based on CPU usage.
    /// * `color` - THE `AlertColor` for the alert.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the CPU trigger.
    // ss[related actor.regeneration-survives]
    pub fn with_mcpu_trigger(&self, bound: Trigger<MCPU>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_mcpu.push((bound, color));
        result
    }

    /// Sets a workload trigger that raises an alert when exceeded.
    ///
    /// # Arguments
    ///
    /// * `bound` - THE trigger condition based on workload.
    /// * `color` - THE `AlertColor` for the alert.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the workload trigger.
    // ss[related actor.regeneration-survives]
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
    // ss[related actor.regeneration-survives]
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
    // ss[related actor.regeneration-survives]
    pub fn with_thread_info(&self) -> Self {
        let mut result = self.clone();
        result.show_thread_info = true;
        result
    }

    /// Sets the stack size for the actor.
    ///
    /// # Arguments
    ///
    /// * `bytes_count` - THE desired stack size in bytes.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the updated stack size.
    // ss[related actor.regeneration-survives]
    pub fn with_stack_size(&self, bytes_count: usize) -> Self {
        let mut result = self.clone();
        result.stack_size = Some(bytes_count);
        result
    }

    /// Converts a generic function into a dynamic callable object.
    ///
    /// # Type Parameters
    ///
    /// * `I` - THE input function type.
    /// * `F` - THE future type returned by the function.
    ///
    /// # Arguments
    ///
    /// * `f` - THE function to convert.
    ///
    /// # Returns
    ///
    /// A boxed dynamic function compatible with `DynCall`.
    // ss[related actor.regeneration-survives]
    pub(crate) fn to_dyn_call<I, F>(f: I) -> DynCall
    where
        I: Fn(SteadyActorShadow) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        Box::new(move |ctx| Box::pin(f(ctx)))
    }

    /// Creates a `SteadyContextArchetype` for actor execution with the specified logic.
    ///
    /// # Type Parameters
    ///
    /// * `F` - THE future returned by the execution logic.
    /// * `I` - THE execution logic function.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - THE execution logic for the actor.
    ///
    /// # Returns
    ///
    /// A `SteadyContextArchetype` configured with the actor's execution logic.
    // ss[related actor.regeneration-survives]
    pub(crate) fn single_actor_exec_archetype<F, I>(
        self,
        build_actor_exec: I,
    ) -> SteadyContextArchetype<DynCall>
    where
        I: Fn(SteadyActorShadow) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        let telemetry_tx = self.telemetry_tx.clone();
        let channel_count = self.channel_count.clone();
        let runtime_state = self.runtime_state.clone();
        let args = self.args.clone();
        let oneshot_shutdown_vec = self.oneshot_shutdown_vec.clone();
        let backplane = self.backplane.clone();
        let dyn_call = Self::to_dyn_call(build_actor_exec);

        let id = self
            .actor_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let immutable_identity =
            ActorIdentity::new(id, self.actor_name.name, self.actor_name.suffix);
        self.actor_catalog.write().push(immutable_identity.clone());

        let immutable_actor_metadata = self.build_actor_metadata(immutable_identity).clone();

        // Pre-register with telemetry to avoid "unknown" labels if the actor hangs at startup
        {
            let mut tx = self.telemetry_tx.write();
            tx.push(CollectorDetail {
                ident: immutable_identity,
                telemetry_take: VecDeque::new(),
            });
        }

        let oneshot_shutdown_vec_for_node = oneshot_shutdown_vec.clone();
        let immutable_node_tx_rx = core_exec::block_on(async move {
            let mut backplane = backplane.lock().await;
            if let Some(pb) = &mut *backplane {
                let (shutdown_tx, shutdown_rx) = oneshot::channel();
                {
                    let mut v: MutexGuard<'_, Vec<Sender<()>>> =
                        oneshot_shutdown_vec_for_node.lock().await;
                    v.push(shutdown_tx);
                }
                pb.register_node(
                    immutable_identity.label,
                    steady_config::BACKPLANE_CAPACITY,
                    shutdown_rx,
                );
                pb.node_tx_rx(immutable_identity.label)
            } else {
                None
            }
        });
        let immutable_oneshot_shutdown = {
            let (send_shutdown_notice, oneshot_shutdown) = oneshot::channel();
            let oneshot_shutdown_vec = oneshot_shutdown_vec.clone();
            let runtime_state = runtime_state.clone();
            core_exec::block_on(async move {
                let mut v: MutexGuard<'_, Vec<Sender<()>>> = oneshot_shutdown_vec.lock().await;
                // If the graph is already in StopRequested state, fire the signal immediately
                // for this new actor instance. This ensures that actors born during the
                // shutdown window (e.g. after a panic) don't miss the global signal.
                if runtime_state
                    .read()
                    .is_in_state(&[GraphLivelinessState::StopRequested])
                {
                    let _ = send_shutdown_notice.send(());
                } else {
                    v.push(send_shutdown_notice);
                }
            });
            oneshot_shutdown.shared()
        };
        let force_internal_behavior_in_test = self.is_for_test
            && self
                .test_pipeline_internal_names
                .contains(self.actor_name.name);
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
            aeron_meda_driver: self.aeron_meda_driver,
            aeron_init_for_tests: self.is_for_test,
            never_simulate: self.never_simulate,
            force_internal_behavior_in_test,
            shutdown_barrier: self.shutdown_barrier,
        }
    }

    /// Constructs actor metadata for telemetry and monitoring.
    ///
    /// # Arguments
    ///
    /// * `ident` - THE unique identifier for the actor.
    ///
    /// # Returns
    ///
    /// An `Arc` containing the actor metadata.
    // ss[related actor.regeneration-survives]
    pub(crate) fn build_actor_metadata(&self, ident: ActorIdentity) -> Arc<ActorMetaData> {
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
