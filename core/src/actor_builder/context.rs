// ss[related actor.regeneration-survives]
use crate::dot::RemoteDetails;
// ss[related philosophy.structural-hierarchy]
use crate::graph_liveliness::{ActorIdentity, GraphLiveliness, GraphLivelinessState};
// ss[related philosophy.structural-hierarchy]
use crate::graph_testing::SideChannel;
// ss[related actor.regeneration-survives]
use crate::monitor::ActorMetaData;
// ss[related philosophy.structural-hierarchy]
use crate::steady_actor_shadow::SteadyActorShadow;
// ss[related philosophy.structural-hierarchy]
use crate::telemetry::metrics_collector::CollectorDetail;
// ss[related actor.regeneration-survives]
use crate::*;
// ss[related philosophy.structural-hierarchy]
use aeron::aeron::Aeron;
// ss[related philosophy.structural-hierarchy]
use async_lock::Barrier;
// ss[related actor.regeneration-survives]
use futures::channel::oneshot::{Receiver, Sender};
// ss[related philosophy.structural-hierarchy]
use futures_util::future::Shared;
// ss[related philosophy.structural-hierarchy]
use futures_util::lock::Mutex;
// ss[related actor.regeneration-survives]
use parking_lot::RwLock;
// ss[related philosophy.structural-hierarchy]
use std::any::Any;
// ss[related philosophy.structural-hierarchy]
use std::collections::VecDeque;
// ss[related actor.regeneration-survives]
use std::error::Error;
// ss[related philosophy.structural-hierarchy]
use std::future::Future;
// ss[related philosophy.structural-hierarchy]
use std::pin::Pin;
// ss[related actor.regeneration-survives]
use std::sync::atomic::{AtomicUsize, Ordering};
// ss[related philosophy.structural-hierarchy]
use std::sync::{Arc, OnceLock};
// ss[related philosophy.structural-hierarchy]
use std::time::Instant;

/// A type alias for a pinned future representing an actor's execution logic.
// ss[related actor.regeneration-survives]
pub(crate) type PinnedFuture = Pin<Box<dyn Future<Output = Result<(), Box<dyn Error>>> + 'static>>;

/// A type alias for a dynamic function that takes a `SteadyActorShadow` and returns a `PinnedFuture`.
// ss[related actor.regeneration-survives]
pub(crate) type DynCall = Box<dyn Fn(SteadyActorShadow) -> PinnedFuture + Send + Sync + 'static>;

/// A type alias for a mutex containing a side-channel transmitter and shutdown receiver, used in testing.
// ss[related actor.regeneration-survives]
pub(crate) type NodeTxRx = Mutex<(SideChannel, Receiver<()>)>;

/// A template for building actor contexts, encapsulating all necessary parameters and state for actor execution.
///
/// This struct serves as a blueprint for creating `SteadyActorShadow` instances, which provide the runtime environment
/// for actors.
// ss[related actor.regeneration-survives]
pub(crate) struct SteadyContextArchetype<DynCall: ?Sized> {
    /// THE execution logic for the actor, wrapped to avoid `Send` requirements.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) build_actor_exec: NonSendWrapper<DynCall>,
    /// Shared liveliness state of the graph.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    /// Shared counter for the number of channels.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) channel_count: Arc<AtomicUsize>,
    /// Unique identifier for the actor.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) ident: ActorIdentity,
    /// Shared arguments for the actor.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) args: Arc<Box<dyn Any + Send + Sync>>,
    /// Telemetry receivers for monitoring.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    /// Metadata for the actor, including telemetry configurations.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) actor_metadata: Arc<ActorMetaData>,
    /// Vector of oneshot senders for shutdown notifications.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<Sender<()>>>>,
    /// A shared future that resolves when a shutdown is requested.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) oneshot_shutdown: Shared<Receiver<()>>,
    /// Optional node transmitter and receiver for side-channel communications.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) node_tx_rx: Option<Arc<NodeTxRx>>,
    /// Flag indicating whether to show thread information in telemetry.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) show_thread_info: bool,
    /// Lazily initialized Aeron media driver.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) aeron_meda_driver: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    /// Short Aeron init retry budget when graph/actor is for testing.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) aeron_init_for_tests: bool,
    /// Flag indicating whether to prevent simulation.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) never_simulate: bool,
    /// When true, test graphs use `internal_behavior` for this actor (see graph name allowlist).
    // ss[related philosophy.structural-hierarchy]
    pub(crate) force_internal_behavior_in_test: bool,
    /// Optional barrier for synchronizing shutdown.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) shutdown_barrier: Option<Arc<Barrier>>,
}

// ss[related actor.regeneration-survives]
impl<T: ?Sized> Clone for SteadyContextArchetype<T> {
    // ss[related philosophy.structural-hierarchy]
    fn clone(&self) -> Self {
        SteadyContextArchetype {
            build_actor_exec: self.build_actor_exec.clone(),
            runtime_state: self.runtime_state.clone(),
            channel_count: self.channel_count.clone(),
            ident: self.ident,
            args: self.args.clone(),
            all_telemetry_rx: self.all_telemetry_rx.clone(),
            actor_metadata: self.actor_metadata.clone(),
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown: self.oneshot_shutdown.clone(),
            node_tx_rx: self.node_tx_rx.clone(),
            show_thread_info: self.show_thread_info,
            aeron_meda_driver: self.aeron_meda_driver.clone(),
            aeron_init_for_tests: self.aeron_init_for_tests,
            never_simulate: self.never_simulate,
            force_internal_behavior_in_test: self.force_internal_behavior_in_test,
            shutdown_barrier: self.shutdown_barrier.clone(),
        }
    }
}

/// A wrapper to handle types that are not `Send` by using `Arc<Mutex<T>>`.
///
/// This allows non-`Send` types to be used safely in multi-threaded contexts by synchronizing access.
// ss[related actor.regeneration-survives]
pub struct NonSendWrapper<T: ?Sized> {
    /// THE inner value wrapped in an `Arc<Mutex<T>>`.
    inner: Arc<Mutex<T>>,
}

// SAFETY: THE wrapper is `Send` because access to `T` is synchronized via `Mutex`.
unsafe impl<T> Send for NonSendWrapper<T> {}

// ss[related actor.regeneration-survives]
impl<T: ?Sized> NonSendWrapper<T> {
    /// Creates a new `NonSendWrapper` instance with the given inner value.
    ///
    /// # Arguments
    ///
    /// * `inner` - THE value to wrap.
    ///
    /// # Returns
    ///
    /// a new `NonSendWrapper` instance.
    // ss[related actor.regeneration-survives]
    pub fn new(inner: T) -> NonSendWrapper<T>
    where
        T: Sized,
    {
        NonSendWrapper {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Asynchronously acquires the guard on the inner value.
    ///
    /// # Returns
    ///
    /// A `MutexGuard` for accessing the inner value.
    // ss[related actor.regeneration-survives]
    pub async fn lock(&self) -> MutexGuard<'_, T> {
        self.inner.lock().await
    }

    /// Guard-first alias for [`NonSendWrapper::lock`] — the preferred spelling.
    ///
    /// Identical guard and semantics; only the vocabulary changes.
    // ss[related actor.regeneration-survives]
    pub async fn acquire_guard(&self) -> MutexGuard<'_, T> {
        self.inner.lock().await
    }

    /// Attempts to lock the inner value immediately, returning a guard if successful.
    ///
    /// # Returns
    ///
    /// An `Option` containing a `MutexGuard` if the lock is acquired, or `None` if it is contended.
    // ss[related actor.regeneration-survives]
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.inner.try_lock()
    }

    /// Clones the wrapper, providing shared ownership of the inner value.
    ///
    /// # Returns
    ///
    /// A new `NonSendWrapper` instance sharing the same inner value.
    // ss[related actor.regeneration-survives]
    pub fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

/// Registers an actor with the graph's liveliness state and returns its execution logic wrapper.
///
/// # Arguments
///
/// * `builder_source` - THE archetype containing the actor's context and logic.
///
/// # Returns
///
/// THE `NonSendWrapper` containing the actor's execution logic.
// ss[related actor.regeneration-survives]
pub(crate) fn build_actor_registration(
    builder_source: &SteadyContextArchetype<DynCall>,
) -> NonSendWrapper<DynCall> {
    builder_source
        .runtime_state
        .write()
        .register_voter(builder_source.ident);
    builder_source.build_actor_exec.clone()
}

/// Removes an actor from the graph's liveliness state upon clean exit.
///
/// # Arguments
///
/// * `builder_source` - THE archetype containing the actor's context and logic.
// ss[related actor.regeneration-survives]
pub(crate) fn exit_actor_registration(builder_source: &SteadyContextArchetype<DynCall>) {
    builder_source
        .runtime_state
        .write()
        .remove_voter(builder_source.ident);
}

/// Constructs a `SteadyActorShadow` context for an actor based on the archetype and parameters.
///
/// # Arguments
///
/// * `builder_source` - THE archetype containing the actor's context and logic.
/// * `frame_rate_ms` - THE frame rate in milliseconds for telemetry.
/// * `team_id` - THE identifier of the team the actor belongs to.
/// * `is_test` - Flag indicating if the actor is for testing.
///
/// # Returns
///
/// A `SteadyActorShadow` instance representing the actor's runtime context.
// ss[related actor.regeneration-survives]
pub(crate) fn build_actor_context<I: ?Sized>(
    builder_source: &SteadyContextArchetype<I>,
    frame_rate_ms: u64,
    team_id: usize,
    is_test: bool,
) -> SteadyActorShadow {
    // ss[impl testing.never-run-in-unit]
    // ss[impl testing.graph-for-testing]
    let use_internal_behavior = builder_source.never_simulate
        || !is_test
        || builder_source.force_internal_behavior_in_test;
    SteadyActorShadow {
        runtime_state: builder_source.runtime_state.clone(),
        channel_count: builder_source.channel_count.clone(),
        ident: builder_source.ident,
        args: builder_source.args.clone(),
        all_telemetry_rx: builder_source.all_telemetry_rx.clone(),
        actor_metadata: builder_source.actor_metadata.clone(),
        oneshot_shutdown_vec: builder_source.oneshot_shutdown_vec.clone(),
        oneshot_shutdown: builder_source.oneshot_shutdown.clone(),
        node_tx_rx: builder_source.node_tx_rx.clone(),
        regeneration: 0u32,
        last_periodic_wait: Default::default(),
        is_in_graph: true,
        actor_start_time: Instant::now(),
        team_id,
        frame_rate_ms,
        show_thread_info: builder_source.show_thread_info,
        aeron_meda_driver: builder_source.aeron_meda_driver.clone(),
        aeron_init_for_tests: builder_source.aeron_init_for_tests,
        use_internal_behavior,
        shutdown_barrier: builder_source.shutdown_barrier.clone(),
        index_wait_last_avail: AtomicUsize::new(usize::MAX),
        index_wait_last_vacant: AtomicUsize::new(usize::MAX),
        index_wait_last_avail_vacant: AtomicUsize::new(usize::MAX),
    }
}
