use std::any::Any;
use std::future::Future;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize};
use std::time::Duration;
use bastion::{Bastion, Callbacks};
use bastion::prelude::SupervisionStrategy;
use bastion::supervisor::SupervisorRef;
use futures::lock::Mutex;
use log::*;
use crate::{ActorIdentity, AlertColor, Graph, GraphLiveliness, MCPU, Percentile, StdDev, SteadyContext, Trigger, Work};
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;

fn build_node_monitor_callback( count_restarts: &Arc<AtomicU32>

) -> Callbacks {
    let callback_count_restarts = count_restarts.clone();

    Callbacks::new()
        .with_after_restart(move || {
            if callback_count_restarts.load(std::sync::atomic::Ordering::Relaxed).lt(&u32::MAX) {
                callback_count_restarts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        })

}

#[derive(Clone)]
pub struct ActorBuilder {
    name: &'static str,
    suffix: Option<usize>,
    args: Arc<Box<dyn Any+Send+Sync>>,
    telemetry_tx: Arc<Mutex<Vec<CollectorDetail>>>,
    channel_count: Arc<AtomicUsize>,
    runtime_state: Arc<Mutex<GraphLiveliness>>,
    redundancy: usize,
    monitor_count: Arc<AtomicUsize>,

    refresh_rate_in_bits: u8,
    window_bucket_in_bits: u8,
    usage_review: bool,

    percentiles_mcpu: Vec<Percentile>,
    percentiles_work: Vec<Percentile>,
    std_dev_mcpu: Vec<StdDev>,
    std_dev_work: Vec<StdDev>,
    trigger_mcpu: Vec<(Trigger<MCPU>,AlertColor)>,
    trigger_work: Vec<(Trigger<Work>,AlertColor)>,
    avg_mcpu: bool,
    avg_work: bool,
    supervisor: SupervisorRef,
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
    pub fn new( graph: &mut Graph) -> ActorBuilder {

        let default_super = Bastion::supervisor(|supervisor| supervisor.with_strategy(SupervisionStrategy::OneForOne))
                            .expect("Internal error, Check if OneForOne is no longer supported?");
        ActorBuilder {
            name: "",
            suffix: None,
            monitor_count: graph.monitor_count.clone(),
            args : graph.args.clone(),
            redundancy: 1,
            telemetry_tx: graph.all_telemetry_rx.clone(),
            channel_count: graph.channel_count.clone(),
            runtime_state: graph.runtime_state.clone(),
            supervisor: default_super,
            refresh_rate_in_bits: 6, // 1<<6 == 64
            window_bucket_in_bits: 5, // 1<<5 == 32

            percentiles_mcpu: Vec::new(),
            percentiles_work: Vec::new(),
            std_dev_mcpu: Vec::new(),
            std_dev_work: Vec::new(),
            trigger_mcpu: Vec::new(),
            trigger_work: Vec::new(),
            avg_mcpu: false,
            avg_work: false,
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
        //we must compute the refresh rate first before we do the window
        let frames_per_refresh = refresh.as_micros() / (1000u128 * crate::config::TELEMETRY_PRODUCTION_RATE_MS as u128);
        let refresh_in_bits = (frames_per_refresh as f32).log2().ceil() as u8;
        let refresh_in_micros = (1000u128<<refresh_in_bits) * crate::config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        //now compute the window based on our new bucket size
        let buckets_per_window:f32 = window.as_micros() as f32 / refresh_in_micros as f32;
        //find the next largest power of 2
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
    pub fn with_compute_refresh_window_bucket_bits(&self
                                                   , refresh_bucket_in_bits:u8
                                                   , window_bucket_in_bits: u8) -> Self {
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


    /// Sets the supervision strategy for the actor, dictating how the system responds to actor failures.
    ///
    /// # Arguments
    ///
    /// * `strategy` - The `SupervisionStrategy` to be applied.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified supervision strategy.
    pub fn with_supervisor(&self, strategy: SupervisionStrategy) -> Self {
        let mut result = self.clone();
        result.supervisor = Bastion::supervisor(|supervisor| supervisor.with_strategy(strategy))
            .expect("Internal error, Check if {strategy} is no longer supported?");
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
        result.avg_mcpu=true;
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
        result.trigger_mcpu.push((bound,color));
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
        result.trigger_work.push((bound,color));
        result
    }

    /// Configures the number of redundant actor instances to be created, enhancing fault tolerance and system resilience.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of redundant instances.
    ///
    /// # Returns
    ///
    /// A new `ActorBuilder` instance with the specified redundancy.
    pub fn with_redundancy(&self, count:usize) -> ActorBuilder {
        let mut result = self.clone();
        result.redundancy = count.max(1);
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
        let frame_ms: u128 = 1000u128 * crate::config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        let est_buckets = millis / frame_ms;
        //find the next largest power of 2
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
    pub fn build_with_exec<F,I>(self, exec: I)
        where
            I: Fn(SteadyContext) -> F + Send + Sync + 'static,
            F: Future<Output = Result<(), ()>> + Send + 'static,
    {

        let id            = self.monitor_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let telemetry_tx  = self.telemetry_tx;
        let channel_count = self.channel_count;
        let runtime_state = self.runtime_state;
        let args          = self.args;
        let redundancy    = self.redundancy;
        let name          = self.name;
        let actor_metadata = Arc::new(
            ActorMetaData {
                id, name,
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
            }
        );

        let _ = self.supervisor.children(|children| {

            let count_restarts = Arc::new(AtomicU32::new(0));
            let watch_node_callback = build_node_monitor_callback(&count_restarts);
            let count_restarts = count_restarts.clone();

            let children = children.with_redundancy(redundancy)
                .with_name(name)//.with_resizer()  TODO: add later.
                .with_callbacks(watch_node_callback);

            #[cfg(debug_assertions)]
            let children = {
                use bastion::distributor::Distributor;
                children.with_distributor(Distributor::named(format!("testing-{name}")))
            };

            children.with_exec(move |ctx| {

                let monitor = SteadyContext {
                                    runtime_state: runtime_state.clone(),
                                    channel_count: channel_count.clone(),
                                    ident: ActorIdentity{id,name},
                                    ctx: Some(ctx),
                                    redundancy,
                                    args: args.clone(),
                                    all_telemetry_rx: telemetry_tx.clone(),
                                    count_restarts: count_restarts.clone(),
                                    actor_metadata: actor_metadata.clone(),
                };
                let future = exec(monitor);

                async move {
                    match future.await {
                        Ok(_) => {
                            trace!("Actor {:?} finished ", name);
                        },
                        Err(e) => {
                            error!("{:?}", e);
                            return Err(e);
                        }
                    }
                    Ok(())
                }
            })
        });

    }
}
