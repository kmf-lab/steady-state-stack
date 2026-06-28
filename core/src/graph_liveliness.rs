//! Compatibility shim — graph types live in [`crate::graph`].
pub use crate::graph::*;

#[cfg(test)]
#[path = "graph_liveliness/tests/mod.rs"]
mod graph_liveliness_tests;
