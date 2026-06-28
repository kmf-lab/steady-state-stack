//! Production orchestration thread builder and entry point.

use std::any::Any;
use std::collections::HashSet;
use std::io;

use crate::graph::{Graph, GraphBuilder};
use crate::logging::{init_logging, LogFileConfig, LogLevel};

/// Builder for the orchestration environment, ensuring sufficient stack size.
#[derive(Clone, Debug)]
// ss[related philosophy.structural-hierarchy]
pub struct SteadyRunner {
    stack_size: usize,
    name: String,
    loglevel: Option<LogLevel>,
    log_file_config: Option<LogFileConfig>,
    default_actor_stack_size: Option<usize>,
    barrier_size: Option<usize>,
    telemetry_rate_ms: Option<u64>,
    telemetry_colors: Option<(String, String)>,
    for_test: bool,
    bundle_floor_size: Option<usize>,
    /// When false (default for [`SteadyRunner::test_build`]), the base name `WORKER` uses real
    /// `internal_behavior` in test graphs. Set to true to disable that default.
    skip_default_test_pipeline_worker: bool,
    /// Additional actor base names (beyond the optional `WORKER` default) that use `internal_behavior` in test graphs.
    extra_test_pipeline_internal_names: HashSet<&'static str>,
}

// ss[related philosophy.structural-hierarchy]
impl SteadyRunner {
    /// Creates a new SteadyRunner with default settings.
    pub fn test_build() -> Self {
        Self {
            stack_size: 16 * 1024 * 1024, // 16 MiB default for main
            name: "steady-orchestrator".to_string(),
            loglevel: None,
            log_file_config: None,
            default_actor_stack_size: Some(2 * 1024 * 1024), // 2 MiB default for each actor
            barrier_size: None,
            telemetry_rate_ms: None,
            telemetry_colors: None,
            for_test: true,
            bundle_floor_size: None,
            skip_default_test_pipeline_worker: false,
            extra_test_pipeline_internal_names: HashSet::new(),
        }
    }
    /// Creates a new SteadyRunner with default settings.
    // ss[related philosophy.structural-hierarchy]
    pub fn release_build() -> Self {
        Self {
            stack_size: 16 * 1024 * 1024, // 16 MiB default for main
            name: "steady-orchestrator".to_string(),
            loglevel: None,
            log_file_config: None,
            default_actor_stack_size: Some(2 * 1024 * 1024), // 2 MiB default for each actor
            barrier_size: None,
            telemetry_rate_ms: None,
            telemetry_colors: None,
            for_test: false,
            bundle_floor_size: None,
            skip_default_test_pipeline_worker: false,
            extra_test_pipeline_internal_names: HashSet::new(),
        }
    }

    /// Sets the stack size for the orchestration thread.
    // ss[related philosophy.structural-hierarchy]
    pub fn with_stack_size(mut self, bytes: usize) -> Self {
        self.stack_size = bytes;
        self
    }

    /// Sets the logging level for the application.
    // ss[related philosophy.structural-hierarchy]
    pub fn with_logging(mut self, level: LogLevel) -> Self {
        self.loglevel = Some(level);
        self
    }

    /// Sets the file logging configuration for the application.
    // ss[related philosophy.structural-hierarchy]
    pub fn with_file_logging(
        mut self,
        directory: &str,
        base_name: &str,
        max_size_bytes: u64,
        keep_count: usize,
        delete_old_on_start: bool,
    ) -> Self {
        self.log_file_config = Some(LogFileConfig {
            directory: directory.to_string(),
            base_name: base_name.to_string(),
            max_size_bytes,
            keep_count,
            delete_old_on_start,
        });
        self
    }

    /// Sets the default actor stack size for the graph.
    // ss[related philosophy.structural-hierarchy]
    pub fn with_default_actor_stack_size(mut self, size: usize) -> Self {
        self.default_actor_stack_size = Some(size);
        self
    }

    /// Sets the size of the shutdown barrier for the graph.
    // ss[related philosophy.structural-hierarchy]
    pub fn with_shutdown_barrier(mut self, size: usize) -> Self {
        self.barrier_size = Some(size);
        self
    }

    /// Sets the telemetry rate for the graph.
    // ss[related philosophy.structural-hierarchy]
    pub fn with_telemetry_rate_ms(mut self, ms: u64) -> Self {
        self.telemetry_rate_ms = Some(ms);
        self
    }

    /// Sets the telemetry top bar colors (primary and secondary hex strings).
    // ss[related philosophy.structural-hierarchy]
    pub fn with_telemetry_colors(mut self, primary_color: &str, secondary_color: &str) -> Self {
        self.telemetry_colors = Some((primary_color.to_string(), secondary_color.to_string()));
        self
    }

    /// Sets the bundle floor size for the graph.
    // ss[related philosophy.structural-hierarchy]
    pub fn with_bundle_floor_size(mut self, size: usize) -> Self {
        self.bundle_floor_size = Some(size);
        self
    }

    /// Do not add the default `WORKER` base name to the test-graph pipeline allowlist (see [`GraphBuilder::with_test_pipeline_internal_behavior_names`]).
    // ss[related philosophy.structural-hierarchy]
    pub fn without_default_test_pipeline_worker(mut self) -> Self {
        self.skip_default_test_pipeline_worker = true;
        self
    }

    /// Names merged into the test-graph allowlist for actors that should run real `internal_behavior`.
    /// The default `WORKER` entry is still added unless [`SteadyRunner::without_default_test_pipeline_worker`] was used.
    // ss[related philosophy.structural-hierarchy]
    pub fn with_test_pipeline_internal_behavior_names(
        mut self,
        names: HashSet<&'static str>,
    ) -> Self {
        self.extra_test_pipeline_internal_names = names;
        self
    }

    /// Spawns a guarded thread, initializes a production graph, and executes the provided closure.
    /// The result (including errors) from the closure is propagated back to the caller as a boxed,
    /// thread-safe error. Panics in the thread are unwound (propagated) to the calling thread.
    // ss[related philosophy.structural-hierarchy]
    pub fn run<A, F>(self, args: A, f: F) -> Result<(), Box<dyn std::error::Error>>
    where
        A: Any + Send + Sync + 'static,
        F: FnOnce(Graph) -> Result<(), Box<dyn std::error::Error>> + std::marker::Send + 'static,
    {
        let builder = std::thread::Builder::new()
            .name(self.name)
            .stack_size(self.stack_size);

        // Spawn the thread and capture its join handle
        let handle = builder
            .spawn(move || {
                // Initialize logging if specified; ignore errors to avoid masking closure failures
                if let Some(level) = self.loglevel {
                    let _ = init_logging(level, self.log_file_config);
                }

                let mut graph = if self.for_test {
                    GraphBuilder::for_testing()
                } else {
                    GraphBuilder::for_production()
                };

                if let Some(size) = self.default_actor_stack_size {
                    graph = graph.with_default_actor_stack_size(size);
                }
                if let Some(size) = self.barrier_size {
                    graph = graph.with_shutdown_barrier(size);
                }
                if let Some(rate) = self.telemetry_rate_ms {
                    graph = graph.with_telemtry_production_rate_ms(rate);
                }
                if let Some((ref c1, ref c2)) = self.telemetry_colors {
                    graph = graph.with_telemetry_colors(c1, c2);
                }
                if let Some(size) = self.bundle_floor_size {
                    graph = graph.with_bundle_floor_size(size);
                }

                if self.for_test {
                    let mut names = HashSet::new();
                    if !self.skip_default_test_pipeline_worker {
                        names.insert("WORKER");
                    }
                    names.extend(self.extra_test_pipeline_internal_names.iter().copied());
                    graph = graph.with_test_pipeline_internal_behavior_names(names);
                }

                let graph = graph.build(args);

                // Execute the user closure and return its result directly
                // (This propagates the Result from f, allowing errors to cross the thread boundary safely)
                match f(graph) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        let err_msg = e.to_string();
                        Err(Box::new(io::Error::new(io::ErrorKind::Other, err_msg))
                            as Box<dyn std::error::Error + Send + Sync + 'static>)
                    }
                }
            })
            .expect("Failed to spawn production orchestrator thread");

        // Block and retrieve the thread's result: Inner is the closure's Result, outer is panic info
        // Unwrap the outer Result; if the thread panicked, resume unwinding to propagate it
        // If successful, return the inner Result (Ok(()) or Err(Box<dyn Error + Send + Sync + 'static>))
        match handle.join() {
            Ok(inner_result) => match inner_result {
                Ok(()) => Ok(()),
                Err(e) => {
                    let err_msg = e.to_string();
                    Err(Box::new(io::Error::new(io::ErrorKind::Other, err_msg))
                        as Box<dyn std::error::Error + Send + Sync + 'static>)
                }
            },
            Err(panic) => {
                // Thread panicked: Resume unwinding to propagate the panic to the caller
                std::panic::resume_unwind(panic);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::SteadyRunner;
    use crate::logging::LogLevel;

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn test_build_run_succeeds_with_builder_chain() {
        SteadyRunner::test_build()
            .with_stack_size(8 * 1024 * 1024)
            .with_logging(LogLevel::Info)
            .with_file_logging("/tmp", "steady", 4096, 2, false)
            .with_default_actor_stack_size(512 * 1024)
            .with_shutdown_barrier(1)
            .with_telemetry_rate_ms(200)
            .with_telemetry_colors("#111111", "#222222")
            .with_bundle_floor_size(16)
            .run((), |_graph| Ok(()))
            .expect("runner ok");
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn test_build_run_propagates_closure_error() {
        let err = SteadyRunner::test_build()
            .run((), |_| Err("runner failure".into()))
            .expect_err("expected error");
        assert!(err.to_string().contains("runner failure"));
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn test_build_pipeline_internal_behavior_names_applied() {
        let mut names = HashSet::new();
        names.insert("CUSTOM");

        SteadyRunner::test_build()
            .without_default_test_pipeline_worker()
            .with_test_pipeline_internal_behavior_names(names)
            .run((), |graph| {
                assert!(!graph
                    .test_pipeline_internal_names
                    .contains("WORKER"));
                assert!(graph.test_pipeline_internal_names.contains("CUSTOM"));
                Ok(())
            })
            .expect("runner ok");
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn release_build_constructs_and_default_worker_allowlist() {
        let _ = SteadyRunner::release_build();
        SteadyRunner::test_build()
            .run((), |graph| {
                assert!(graph.test_pipeline_internal_names.contains("WORKER"));
                Ok(())
            })
            .expect("default WORKER allowlist");
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn test_build_runs_minimal_graph_shutdown() {
        SteadyRunner::test_build().run((), |mut graph| {
            graph.start();
            graph.request_shutdown();
            graph.block_until_stopped(std::time::Duration::from_secs(2))
        })
        .expect("clean shutdown");
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn file_logging_without_level_still_runs() {
        SteadyRunner::test_build()
            .with_file_logging("/tmp", "no_level", 2048, 1, false)
            .run((), |_graph| Ok(()))
            .expect("runner ok without loglevel");
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn release_build_constructs_without_running_production_graph() {
        let _runner = SteadyRunner::release_build()
            .with_stack_size(4 * 1024 * 1024)
            .with_default_actor_stack_size(512 * 1024)
            .with_shutdown_barrier(2);
    }
}

#[cfg(test)]
#[path = "runner_proptest.rs"]
mod runner_proptest;
