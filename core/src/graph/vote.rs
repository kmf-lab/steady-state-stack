use super::deps::*;
use super::identity::ActorIdentity;

/// Represents a vote cast by an actor regarding the shutdown of the graph.
///
/// This struct encapsulates the details of an actor's decision during the shutdown voting process,
/// including their identity and reasoning if they oppose the shutdown.
#[derive(Default)]
// ss[related graph.for-testing]
pub struct ShutdownVote {
    /// THE unique identifier of the actor casting the vote.
    pub(crate) id: usize,
    /// THE optional identity of the actor, providing additional context.
    pub(crate) signature: Option<ActorIdentity>,
    /// Indicates whether the actor supports the shutdown.
    pub(crate) in_favor: bool,
    /// THE current status of the voter, such as registered or dead.
    pub(crate) voter_status: VoterStatus,
    /// An optional backtrace captured if the actor vetoes the shutdown, useful for debugging.
    pub(crate) veto_backtrace: Option<Backtrace>,
    /// An optional reason provided by the actor for vetoing the shutdown.
    pub(crate) veto_reason: Option<Eye>,
}

/// Indicates the status of an actor in the voting process.
///
/// This enum defines whether an actor is actively registered, marked as dead, or not yet registered,
/// affecting its participation in shutdown votes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
// ss[related graph.for-testing]
pub(crate) enum VoterStatus {
    /// THE actor has not yet registered as a voter.
    #[default]
    None,
    /// THE actor is registered and eligible to vote.
    Registered(ActorIdentity),
    /// THE actor is marked as dead and cannot participate in voting.
    Dead(ActorIdentity),
}
