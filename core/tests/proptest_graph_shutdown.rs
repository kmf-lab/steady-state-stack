//! Wave 4 full-graph shutdown proptest.
//!
//! Implementation: `graph_testing_tests::proptest_pipeline_random_messages_clean_shutdown`
//! (worker uses `pipeline_worker_internal` directly — no `run()`; edge actors are `never_simulate` puppets).
//!
//! Run:
//! `cargo nextest run --profile ci-unit --no-default-features --features exec_async_std,prometheus_metrics proptest_pipeline_random_messages_clean_shutdown`
