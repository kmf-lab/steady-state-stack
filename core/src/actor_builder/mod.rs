//! THE `actor_builder` module provides structures and functions to create, configure, and manage actors within a system.
//! This module includes the `ActorBuilder` for building actors, `Troupe` for managing groups of actors, and various utility
//! functions and types to support actor creation and telemetry monitoring.

// ss[related actor.regeneration-survives]
mod affinity;
// ss[related philosophy.structural-hierarchy]
mod builder;
// ss[related philosophy.structural-hierarchy]
mod context;
// ss[impl troupe.dynamic-kruskal-capacity]
pub(crate) mod pack;
// ss[impl troupe.dynamic-slot-budget]
pub(crate) mod slot_budget;
// ss[related actor.regeneration-survives]
mod spawn;
// ss[related philosophy.structural-hierarchy]
mod troupe;

#[cfg(test)]
// ss[related actor.regeneration-survives]
mod tests;

// ss[related philosophy.structural-hierarchy]
pub use affinity::CoreBalancer;
// ss[related actor.regeneration-survives]
pub use builder::ActorBuilder;
// ss[related philosophy.structural-hierarchy]
pub(crate) use context::NodeTxRx;
// ss[related philosophy.structural-hierarchy]
pub use context::NonSendWrapper;
// ss[related actor.regeneration-survives]
pub use spawn::{launch_actor, ScheduleAs};
// ss[impl troupe.dynamic-is-a-bag]
pub use troupe::{Troupe, TroupeGuard, TroupeKind};
// ss[impl troupe.dynamic-finalize-at-start]
pub(crate) use troupe::FutureBuilderType;
// ss[impl troupe.dynamic-slot-budget]
pub use slot_budget::SlotBudget;

// Re-export test/support items for integration tests in tests.rs
#[cfg(test)]
// ss[related actor.regeneration-survives]
pub(crate) use context::SteadyContextArchetype;
