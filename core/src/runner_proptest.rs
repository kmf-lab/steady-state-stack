//! Property tests for [`SteadyRunner`] orchestration and test-graph allowlists.

use super::SteadyRunner;
use crate::graph::GraphBuilder;
use crate::logging::LogLevel;
use crate::ss_proptest;
use proptest::prelude::*;
use std::collections::HashSet;
use std::time::Duration;

ss_proptest! {

    /// Property: `SteadyRunner::test_build` runs an empty graph to clean shutdown.
    #[test]
    // ss[verify graph.for-testing]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_runner_empty_graph_shutdown(timeout_ms in 50u64..2_000) {
        SteadyRunner::test_build()
            .run((), move |mut graph| {
                graph.start();
                graph.request_shutdown();
                graph.block_until_stopped(Duration::from_millis(timeout_ms))
            })
            .expect("runner clean shutdown");
    }

    /// Property: default test runner includes `WORKER` on the internal-behavior allowlist.
    #[test]
    // ss[verify testing.pipeline-worker-allowlist]
    // ss[verify verify.process.proptest]
    fn proptest_runner_default_worker_allowlist(_seed in 0u8..4) {
        SteadyRunner::test_build()
            .run((), |graph| {
                if graph.test_pipeline_internal_names.contains("WORKER") {
                    Ok(())
                } else {
                    Err("WORKER missing from allowlist".into())
                }
            })
            .expect("runner ok");
    }

    /// Property: `without_default_test_pipeline_worker` removes `WORKER` from the allowlist.
    #[test]
    // ss[verify testing.pipeline-worker-allowlist]
    // ss[verify verify.process.proptest]
    fn proptest_runner_without_default_worker(
        extra in prop::sample::select(vec!["CUSTOM", "PIPE", "EDGE"]),
    ) {
        let mut names = HashSet::new();
        names.insert(extra);
        SteadyRunner::test_build()
            .without_default_test_pipeline_worker()
            .with_test_pipeline_internal_behavior_names(names)
            .run((), move |graph| {
                if graph.test_pipeline_internal_names.contains("WORKER") {
                    return Err("WORKER should be absent".into());
                }
                if !graph.test_pipeline_internal_names.contains(extra) {
                    return Err("extra name missing from allowlist".into());
                }
                Ok(())
            })
            .expect("runner ok");
    }

    /// Property: builder chain options compose without panicking.
    #[test]
    // ss[verify philosophy.structural-hierarchy]
    // ss[verify verify.process.proptest]
    fn proptest_runner_builder_chain_runs(
        stack_mb in 4usize..32,
        actor_stack_kb in 256usize..2048,
        telemetry_ms in 40u64..500,
        barrier in 1usize..4,
    ) {
        SteadyRunner::test_build()
            .with_stack_size(stack_mb * 1024 * 1024)
            .with_logging(LogLevel::Info)
            .with_default_actor_stack_size(actor_stack_kb * 1024)
            .with_shutdown_barrier(barrier)
            .with_telemetry_rate_ms(telemetry_ms)
            .with_telemetry_colors("#101010", "#202020")
            .with_bundle_floor_size(8)
            .run((), |_graph| Ok(()))
            .expect("builder chain ok");
    }

    /// Property: closure errors propagate through `run` as failures.
    #[test]
    // ss[verify philosophy.structural-hierarchy]
    // ss[verify verify.process.proptest]
    fn proptest_runner_propagates_closure_error(msg in "\\PC{1,32}") {
        let expected = msg.clone();
        let err = SteadyRunner::test_build()
            .run((), move |_| Err(msg.into()))
            .expect_err("expected error");
        prop_assert!(err.to_string().contains(&expected));
    }

    /// Property: `release_build` constructs with configurable stack and barrier sizes.
    #[test]
    // ss[verify philosophy.structural-hierarchy]
    // ss[verify verify.process.proptest]
    fn proptest_release_build_constructs(
        stack_mb in 8usize..64,
        barrier in 1usize..4,
    ) {
        let _runner = SteadyRunner::release_build()
            .with_stack_size(stack_mb * 1024 * 1024)
            .with_shutdown_barrier(barrier);
    }
}
