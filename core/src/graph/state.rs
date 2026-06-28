/// Represents the possible states of the graph's liveliness within the SteadyState framework.
///
/// This enum tracks the lifecycle of a graph, from its construction through to its shutdown,
/// reflecting the operational status of its actors.
#[derive(PartialEq, Eq, Debug, Clone)]
// ss[related graph.for-testing]
pub enum GraphLivelinessState {
    /// Indicates that the graph is in the process of being constructed.
    ///
    /// During this phase, actors are being added and initialized, and the graph is not yet operational.
    Building,

    /// Indicates that the graph is fully operational and running.
    ///
    /// All actors are actively executing their designated tasks concurrently.
    Running,

    /// Indicates that a shutdown has been requested and actors are voting on it.
    ///
    /// THE graph is transitioning to a stopped state, awaiting consensus from all actors.
    StopRequested,

    /// Indicates that the graph has completely stopped cleanly.
    ///
    /// All actors have ceased execution in an orderly manner.
    Stopped,

    /// Indicates that the graph has stopped, but not all actors shut down cleanly.
    ///
    /// Some actors encountered issues during the shutdown process, leading to an incomplete stop.
    StoppedUncleanly,
}
