//! Graph construction, liveliness, and shutdown orchestration.

// ss[related graph.for-testing]
mod deps;
// ss[related philosophy.structural-hierarchy]
mod identity;
// ss[related philosophy.structural-hierarchy]
mod state;
// ss[related graph.for-testing]
mod vote;
// ss[related philosophy.structural-hierarchy]
mod liveliness;
// ss[related philosophy.structural-hierarchy]
mod builder;
// ss[related graph.for-testing]
mod graph;
// ss[related philosophy.structural-hierarchy]
mod shutdown;
// ss[related philosophy.structural-hierarchy]
mod testing_guard;

// ss[related graph.for-testing]
pub use identity::*;
// ss[related philosophy.structural-hierarchy]
pub use state::*;
// ss[related philosophy.structural-hierarchy]
pub use vote::*;
// ss[related graph.for-testing]
pub use liveliness::*;
// ss[related philosophy.structural-hierarchy]
pub use builder::*;
// ss[related philosophy.structural-hierarchy]
pub(crate) use builder::MIN_MS_RATE;
// ss[related graph.for-testing]
pub use graph::*;
// ss[related philosophy.structural-hierarchy]
pub use shutdown::*;
// ss[related philosophy.structural-hierarchy]
pub use testing_guard::*;
