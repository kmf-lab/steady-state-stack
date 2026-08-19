//! Compatibility shim — graph types live in [`crate::graph`].
// ss[related graph.for-testing]
pub use crate::graph::*;

#[cfg(test)]
#[path = "graph_liveliness/tests/mod.rs"]
// ss[related graph.for-testing]
mod graph_liveliness_tests;
