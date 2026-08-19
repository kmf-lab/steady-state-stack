//! THE `actor_builder` module provides structures and functions to create, configure, and manage actors within a system.
//! This module includes the `ActorBuilder` for building actors, `Troupe` for managing groups of actors, and various utility
//! functions and types to support actor creation and telemetry monitoring.

// ss[related actor.regeneration-survives]
mod affinity;
// ss[related philosophy.structural-hierarchy]
mod builder;
// ss[related philosophy.structural-hierarchy]
mod context;
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
// ss[related philosophy.structural-hierarchy]
pub use troupe::{Troupe, TroupeGuard};

// Re-export test/support items for integration tests in tests.rs
// ss[related actor.regeneration-survives]
pub(crate) use context::{build_actor_context, build_actor_registration, DynCall, SteadyContextArchetype};
