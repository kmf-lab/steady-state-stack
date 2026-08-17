use super::deps::*;
use super::graph::Graph;
use super::liveliness::GraphLiveliness;
use super::shutdown::watch_shutdown;
use super::state::GraphLivelinessState;
use log::{debug, trace, warn};

/// Configures and builds a `Graph` instance with customizable options.
///
/// This struct allows setting up the graph for either production or testing environments, adjusting
/// parameters like telemetry and I/O behavior.
#[derive(Clone, Debug)]
// ss[related graph.for-testing]
pub struct GraphBuilder {
    /// Indicates whether the graph is intended for testing purposes.
    pub(crate) is_for_testing: bool,
    /// Enables or disables telemetry metric features.
    pub(crate) telemetry_metric_features: bool,
    /// An optional backplane for testing side-channel communications.
    pub(crate) backplane: Option<StageManager>,
    /// THE rate at which telemetry data is produced, in milliseconds.
    pub(crate) telemtry_production_rate_ms: u64,
    /// An optional hex color for the telemetry top bar.
    pub(crate) telemetry_colors: Option<(String, String)>,
    /// An optional barrier for synchronizing actor shutdown.
    pub(crate) shutdown_barrier: Option<Arc<Barrier>>,
    /// Default stack size for all actors in the graph.
    pub(crate) default_stack_size: Option<usize>,
    /// Flag to block fail-fast behavior during tests.
    pub(crate) block_fail_fast: bool,
    /// Minimum size for bundles.
    pub(crate) bundle_floor_size: usize,
    /// Actor base names that use real `internal_behavior` in test graphs (StageManager on edges).
    pub(crate) test_pipeline_internal_names: HashSet<&'static str>,
}

// ss[related graph.for-testing]
impl Default for GraphBuilder {
    /// Provides a default `GraphBuilder` configured for production use.
    ///
    /// This implementation returns a builder with production-ready settings.
    // ss[related graph.for-testing]
    fn default() -> Self {
        GraphBuilder::for_production()
    }
}

// ss[related graph.for-testing]
pub(crate) const MIN_MS_RATE: u64 = 100;

// ss[related graph.for-testing]
impl GraphBuilder {
    /// Creates a `GraphBuilder` configured for production environments.
    ///
    /// This method sets up a builder with defaults optimized for production use, such as enabling the I/O driver.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance tailored for production.
    // ss[related graph.for-testing]
    pub fn for_production() -> Self {
        #[cfg(test)]
        panic!("should not call for_production in tests");
        #[cfg(not(test))]
        GraphBuilder {
            is_for_testing: false,
            telemetry_metric_features: crate::steady_config::TELEMETRY_SERVER,
            backplane: None,
            telemtry_production_rate_ms: MIN_MS_RATE,
            telemetry_colors: None,
            shutdown_barrier: None,
            default_stack_size: None,
            block_fail_fast: false,
            bundle_floor_size: 4,
            test_pipeline_internal_names: HashSet::new(),
        }
    }

    /// Creates a `GraphBuilder` configured for testing environments.
    ///
    /// This method sets up a builder with defaults suitable for testing, including a backplane for side channels.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance tailored for testing.
    // ss[impl graph.for-testing]
    // ss[impl testing.graph-for-testing]
    // ss[impl testing.mock-main-thread]
    pub fn for_testing() -> Self {
        let _ = logging_util::steady_logger::initialize();
        GraphBuilder {
            is_for_testing: true,
            telemetry_metric_features: false,
            backplane: Some(StageManager::default()),
            telemtry_production_rate_ms: MIN_MS_RATE,
            telemetry_colors: None,
            shutdown_barrier: None,
            default_stack_size: None,
            block_fail_fast: true,
            bundle_floor_size: 4,
            test_pipeline_internal_names: HashSet::new(),
        }
    }

    /// Replaces the set of actor base names that run real `internal_behavior` in **test** graphs
    /// (for pipeline processors while edges use StageManager simulation). Empty clears the set.
    // ss[related graph.for-testing]
    pub fn with_test_pipeline_internal_behavior_names(
        &self,
        names: HashSet<&'static str>,
    ) -> Self {
        let mut result = self.clone();
        result.test_pipeline_internal_names = names;
        result
    }

    /// Sets the telemetry production rate in milliseconds.
    ///
    /// Values below the internal minimum (`MIN_MS_RATE`, currently 100 ms) are clamped to that minimum
    /// and a warning is logged.
    ///
    /// # Arguments
    ///
    /// * `ms` - THE desired production rate in milliseconds.
    ///
    /// # Returns
    ///
    /// A new `GraphBuilder` instance with the updated telemetry rate.
    // ss[related graph.for-testing]
    pub fn with_telemtry_production_rate_ms(&self, ms: u64) -> Self {
        let mut result = self.clone();
        if ms >= MIN_MS_RATE {
            result.telemtry_production_rate_ms = ms;
        } else {
            if cfg!(test) {
                debug!(
                    "telemetry production rate requested {}ms below minimum {}ms, using {}ms",
                    ms, MIN_MS_RATE, MIN_MS_RATE
                );
            } else {
                warn!(
                    "telemetry production rate must be at least {}ms, using {}ms",
                    MIN_MS_RATE, MIN_MS_RATE
                );
            }
            result.telemtry_production_rate_ms = MIN_MS_RATE;
        }
        result
    }

    /// Sets the telemetry top bar colors (primary and secondary hex strings).
    // ss[related graph.for-testing]
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
    // ss[related graph.for-testing]
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
    // ss[related graph.for-testing]
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
    // ss[related graph.for-testing]
    pub fn with_block_fail_fast(&self) -> Self {
        let mut result = self.clone();
        result.block_fail_fast = true;
        result
    }

    /// Sets the threshold for bundling edges in the telemetry visualization.
    // ss[related graph.for-testing]
    pub fn with_aggregation_threshold(&self, threshold: usize) -> Self {
        let mut result = self.clone();
        result.bundle_floor_size = threshold;
        result
    }

    /// Sets the minimum size for bundles.
    // ss[related graph.for-testing]
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
    // ss[related graph.for-testing]
    pub fn with_telemetry_metric_features(&self, enable: bool) -> Self {
        let mut result = self.clone();
        result.telemetry_metric_features = enable;
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
    // ss[related graph.for-testing]
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
            let _ = watch_shutdown(timeout, now, ctrlc_runtime_state.clone(), tel_prod_rate);
        });
        if let Err(e) = result {
            trace!("Error setting up CTRL-C hook: {}", e);
        }
        g
    }
}

#[cfg(test)]
#[path = "builder_proptest.rs"]
mod builder_proptest;
