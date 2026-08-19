// ss[related graph.actor-identity]
use super::deps::*;
// ss[related philosophy.structural-hierarchy]
use std::fmt::Debug;

/// Identifies an actor within the graph uniquely.
///
/// This struct combines a numeric identifier with a human-readable name for actor distinction.
#[derive(Clone, Default, Copy, PartialEq, Eq, Hash)]
// ss[impl graph.actor-identity]
pub struct ActorIdentity {
    /// A unique numeric identifier for the actor within the graph.
    pub id: usize,
    /// THE human-readable name of the actor, potentially with a suffix for uniqueness.
    pub label: ActorName,
}

/// Represents the name of an actor, including an optional suffix for uniqueness.
///
/// This struct provides a static base name and an optional numeric suffix to differentiate actors.
#[derive(Clone, Default, Copy, PartialEq, Eq, Hash, Debug)]
// ss[related graph.for-testing]
pub struct ActorName {
    /// THE static, immutable base name of the actor.
    pub name: &'static str,
    /// An optional numeric suffix to ensure uniqueness among actors with the same base name.
    pub suffix: Option<usize>,
}

// ss[related graph.for-testing]
impl ActorIdentity {
    /// Constructs a new `ActorIdentity` with the specified parameters.
    ///
    /// This method creates an identity for an actor using a unique ID and a name with an optional suffix.
    ///
    /// # Arguments
    ///
    /// * `id` - THE unique numeric identifier for the actor.
    /// * `name` - THE static base name of the actor.
    /// * `suffix` - An optional numeric suffix for uniqueness.
    ///
    /// # Returns
    ///
    /// A new `ActorIdentity` instance.
    // ss[related graph.for-testing]
    pub fn new(id: usize, name: &'static str, suffix: Option<usize>) -> Self {
        ActorIdentity {
            id,
            label: ActorName { name, suffix },
        }
    }
}

// ss[related graph.for-testing]
impl ActorName {
    /// Constructs a new `ActorName` with the specified name and optional suffix.
    ///
    /// This method creates a name structure for an actor, allowing for differentiation via a suffix.
    ///
    /// # Arguments
    ///
    /// * `name` - THE static base name of the actor.
    /// * `suffix` - An optional numeric suffix for uniqueness.
    ///
    /// # Returns
    ///
    /// A new `ActorName` instance.
    // ss[related graph.for-testing]
    pub fn new(name: &'static str, suffix: Option<usize>) -> Self {
        ActorName { name, suffix }
    }
}

// ss[related graph.for-testing]
impl std::fmt::Debug for ActorIdentity {
    /// Formats the `ActorIdentity` for debugging purposes.
    ///
    /// This implementation provides a string representation including the ID and name, with suffix if present.
    // ss[related graph.for-testing]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:?} {}", self.id, self.label.name)?;
        if let Some(suffix) = self.label.suffix {
            write!(f, "-{}", suffix)?;
        }
        Ok(())
    }
}
