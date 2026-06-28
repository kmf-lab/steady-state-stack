//! THE `actor_builder` module provides structures and functions to create, configure, and manage actors within a system.
//! This module includes the `ActorBuilder` for building actors, `Troupe` for managing groups of actors, and various utility
//! functions and types to support actor creation and telemetry monitoring.

// ss[related actor.regeneration-survives]
mod affinity;
mod builder;
mod context;
mod spawn;
mod troupe;

#[cfg(test)]
mod tests;

pub use affinity::CoreBalancer;
pub use builder::ActorBuilder;
pub(crate) use context::NodeTxRx;
pub use context::NonSendWrapper;
pub use spawn::{launch_actor, ScheduleAs};
pub use troupe::{Troupe, TroupeGuard};

// Re-export test/support items for integration tests in tests.rs
pub(crate) use context::{build_actor_context, build_actor_registration, DynCall, SteadyContextArchetype};
