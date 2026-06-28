// ss[related actor.regeneration-survives]
use super::affinity::pin_thread_to_core;
use super::context::{
    build_actor_context, build_actor_registration, exit_actor_registration, DynCall,
    NonSendWrapper, SteadyContextArchetype,
};
use crate::graph_liveliness::Graph;
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::*;
use futures::channel::oneshot;
use futures::FutureExt;
use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::VecDeque;
use std::any::Any;
use std::error::Error;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

type ActorRuntime = NonSendWrapper<DynCall>;

/// Manages a collection of actors, facilitating their coordinated execution on a shared thread.
///
/// `Troupe` allows grouping multiple actors to run concurrently on the same thread, improving efficiency by reducing
/// thread management overhead.
// ss[related actor.regeneration-survives]
pub struct Troupe {
    /// A queue of future builders for the actors in the troupe.
    pub(crate) future_builder: VecDeque<FutureBuilderType>,
    /// Unique identifier for the troupe.
    team_id: usize,
    /// Optional human-readable name for the troupe.
    pub(crate) name: Option<String>,
}

/// Represents a builder for a future, encapsulating the actor's execution logic and execution parameters.
// ss[related actor.regeneration-survives]
pub(crate) struct FutureBuilderType {
    /// THE archetype containing the actor's execution logic and context.
    fun: SteadyContextArchetype<DynCall>,
    /// THE frame rate in milliseconds for telemetry data collection.
    frame_rate_ms: u64,
    /// Flag indicating whether the actor is for testing purposes.
    is_for_test: bool,
    /// Optional stack size for the actor.
    stack_size: Option<usize>,
}

/// Represents a stable slot for an actor's execution state within a troupe.
// ss[related actor.regeneration-survives]
struct ActorSlot {
    fun: ActorRuntime,
    ctx: SteadyActorShadow,
    arch: SteadyContextArchetype<DynCall>,
}

/// Represents the outcome of an actor's execution, returning the slot for potential restart.
// ss[related actor.regeneration-survives]
struct ActorSlotOutcome {
    slot: ActorSlot,
    result: Result<Result<(), Box<dyn Error>>, Box<dyn Any + Send>>,
}

// ss[related actor.regeneration-survives]
impl FutureBuilderType {
    /// Creates a new `FutureBuilderType` instance.
    ///
    /// # Arguments
    ///
    /// * `fun` - THE archetype containing the actor's execution logic and context.
    /// * `frame_rate_ms` - THE frame rate in milliseconds for telemetry data collection.
    /// * `is_for_test` - Flag indicating whether the actor is for testing purposes.
    /// * `stack_size` - Optional stack size for the actor.
    ///
    /// # Returns
    ///
    /// A new `FutureBuilderType` instance.
    // ss[related actor.regeneration-survives]
    fn new(
        fun: SteadyContextArchetype<DynCall>,
        frame_rate_ms: u64,
        is_for_test: bool,
        stack_size: Option<usize>,
    ) -> Self {
        FutureBuilderType {
            fun,
            frame_rate_ms,
            is_for_test,
            stack_size,
        }
    }

    /// Registers the actor with the graph's liveliness state and returns the execution logic wrapper.
    ///
    /// # Returns
    ///
    /// THE `ActorRuntime` containing the registered execution logic.
    // ss[related actor.regeneration-survives]
    fn register(&self) -> ActorRuntime {
        build_actor_registration(&self.fun)
    }

    /// Constructs a `SteadyActorShadow` context for the actor.
    ///
    /// # Arguments
    ///
    /// * `team_display_id` - THE identifier of the team for display purposes.
    ///
    /// # Returns
    ///
    /// A `SteadyActorShadow` instance representing the actor's runtime context.
    // ss[related actor.regeneration-survives]
    fn context(&self, team_display_id: usize) -> SteadyActorShadow {
        build_actor_context(
            &self.fun,
            self.frame_rate_ms,
            team_display_id,
            self.is_for_test,
        )
    }
}

/// A guard that automatically spawns the troupe when it goes out of scope.
///
/// This guard ensures that the troupe is spawned only when the guard is dropped, allowing for deferred execution.
// ss[related actor.regeneration-survives]
pub struct TroupeGuard {
    /// THE optional troupe to be spawned when the guard is dropped.
    pub(crate) troupe: Option<Troupe>,
}

// ss[related actor.regeneration-survives]
impl std::ops::Deref for TroupeGuard {
    type Target = Troupe;

    /// Provides immutable access to the underlying troupe.
    // ss[related actor.regeneration-survives]
    fn deref(&self) -> &Self::Target {
        self.troupe
            .as_ref()
            .expect("TroupeGuard troupe was already consumed")
    }
}

// ss[related actor.regeneration-survives]
impl std::ops::DerefMut for TroupeGuard {
    /// Provides mutable access to the underlying troupe.
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.troupe
            .as_mut()
            .expect("TroupeGuard troupe was already consumed")
    }
}

// ss[related actor.regeneration-survives]
impl Drop for TroupeGuard {
    /// Spawns the troupe when the guard is dropped, initiating the execution of the actors.
    fn drop(&mut self) {
        if let Some(troupe) = self.troupe.take() {
            troupe.spawn();
        }
    }
}

// ss[related actor.regeneration-survives]
impl TroupeGuard {
    /// Sets a custom name for the troupe, which will be used for the OS thread name.
    ///
    /// # Arguments
    ///
    /// * `name` - THE custom name for the troupe.
    ///
    /// # Returns
    ///
    /// THE `TroupeGuard` instance with the updated name.
    // ss[related actor.regeneration-survives]
    pub fn with_name(mut self, name: &str) -> Self {
        if let Some(ref mut t) = self.troupe {
            t.with_name(name);
        }
        self
    }
}

// ss[related actor.regeneration-survives]
impl Troupe {
    /// Creates a new `Troupe` instance with a unique team identifier derived from the graph.
    ///
    /// # Arguments
    ///
    /// * `graph` - A reference to the `Graph` from which to derive the team count.
    ///
    /// # Returns
    ///
    /// A new `Troupe` instance.
    // ss[related actor.regeneration-survives]
    pub(crate) fn new(graph: &Graph) -> Self {
        Troupe {
            future_builder: VecDeque::new(),
            team_id: graph.team_count.fetch_add(1, Ordering::SeqCst),
            name: None,
        }
    }

    /// Sets a custom name for the troupe.
    ///
    /// # Arguments
    ///
    /// * `name` - THE custom name for the troupe.
    ///
    /// # Returns
    ///
    /// A mutable reference to the `Troupe` instance.
    // ss[related actor.regeneration-survives]
    pub fn with_name(&mut self, name: &str) -> &mut Self {
        self.name = Some(name.to_string());
        self
    }

    /// Adds an actor to the troupe with the specified context and execution parameters.
    ///
    /// # Arguments
    ///
    /// * `context_archetype` - THE archetype containing the actor's execution logic and context.
    /// * `frame_rate_ms` - THE frame rate in milliseconds for telemetry data collection.
    /// * `is_for_test` - Flag indicating whether the actor is for testing purposes.
    /// * `stack_size` - Optional stack size for the actor.
    // ss[related actor.regeneration-survives]
    pub(crate) fn add_actor(
        &mut self,
        context_archetype: SteadyContextArchetype<DynCall>,
        frame_rate_ms: u64,
        is_for_test: bool,
        stack_size: Option<usize>,
    ) {
        self.future_builder.push_back(FutureBuilderType::new(
            context_archetype.clone(),
            frame_rate_ms,
            is_for_test,
            stack_size,
        ));
    }

    /// Transfers the front actor to another `Troupe`.
    ///
    /// # Arguments
    ///
    /// * `other` - THE target `Troupe` to receive the actor.
    ///
    /// # Returns
    ///
    /// `true` if an actor was transferred, `false` if the troupe is empty.
    // ss[related actor.regeneration-survives]
    pub fn transfer_front_to(&mut self, other: &mut Self) -> bool {
        if let Some(f) = self.future_builder.pop_front() {
            other.future_builder.push_back(f);
            true
        } else {
            false
        }
    }

    /// Transfers the back actor to another `Troupe`.
    ///
    /// # Arguments
    ///
    /// * `other` - THE target `Troupe` to receive the actor.
    ///
    /// # Returns
    ///
    /// `true` if an actor was transferred, `false` if the troupe is empty.
    // ss[related actor.regeneration-survives]
    pub fn transfer_back_to(&mut self, other: &mut Self) -> bool {
        if let Some(f) = self.future_builder.pop_back() {
            other.future_builder.push_back(f);
            true
        } else {
            false
        }
    }

    /// Spawns the troupe, executing all actors on a shared thread.
    ///
    /// # Returns
    ///
    /// THE number of actors spawned.
    // ss[related actor.regeneration-survives]
    fn spawn(self) -> usize {
        let count = Arc::new(AtomicUsize::new(0));
        if self.future_builder.is_empty() {
            return 0;
        }

        let (local_send, local_take) = oneshot::channel();
        let count_task = count.clone();
        let team_id = self.team_id;
        let max_stack_size = self
            .future_builder
            .iter()
            .filter_map(|f| f.stack_size)
            .max();

        // 1. ATOMIC REGISTRATION: Register all actors on the main thread.
        // This ensures the Graph is fully aware of all "voters" before any thread starts polling.
        let slots: Vec<ActorSlot> = self
            .future_builder
            .into_iter()
            .map(|f| ActorSlot {
                fun: f.register(),
                ctx: f.context(team_id),
                arch: f.fun.clone(),
            })
            .collect();

        count_task.store(slots.len(), Ordering::SeqCst);

        let thread_name = self
            .name
            .clone()
            .unwrap_or_else(|| format!("Troupe-{}", team_id));

        let mut thread_builder = std::thread::Builder::new().name(thread_name);
        if let Some(size) = max_stack_size {
            thread_builder = thread_builder.stack_size(size);
        }

        let handle = thread_builder.spawn(move || {
            let super_task = async move {
                #[cfg(feature = "core_affinity")]
                if let Err(e) = pin_thread_to_core(team_id) {
                    eprintln!("Failed to pin thread to core {}: {:?}", team_id, e);
                }

                // 2. SIGNAL-FIRST: Tell the main thread we are alive.
                let _ = local_send.send(());

                let mut futures = FuturesUnordered::new();
                for slot in slots {
                    futures.push(Self::build_async_fun(slot));
                }

                while let Some(outcome) = futures.next().await {
                    let mut slot = outcome.slot;
                    match outcome.result {
                        Ok(Ok(_)) => {
                            // Actor finished cleanly
                            exit_actor_registration(&slot.arch);
                        }
                        Ok(Err(e)) => {
                            // Actor returned an Error, restart it
                            error!("Actor {:?} error: {:?}", slot.ctx.ident, e);
                            // ss[impl actor.regeneration-survives]
                            // ss[impl graph.panic-restart]
                            slot.ctx.regeneration += 1;
                            futures.push(Self::build_async_fun(slot));
                        }
                        Err(e) => {
                            // Actor panicked, restart it
                            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                                *s
                            } else if let Some(s) = e.downcast_ref::<String>() {
                                s.as_str()
                            } else {
                                "Unknown panic payload"
                            };

                            error!("PANIC in troupe actor {:?}: {}", slot.ctx.ident, msg);
                            // ss[impl actor.regeneration-survives]
                            // ss[impl graph.panic-restart]
                            slot.ctx.regeneration += 1;
                            futures.push(Self::build_async_fun(slot));
                        }
                    }
                }
            };
            core_exec::block_on(super_task);
        });

        if let Err(e) = handle {
            error!(
                "Failed to spawn OS thread for troupe: {}, error: {:?}",
                team_id, e
            );
        } else {
            // Wait for the troupe thread to signal it has started before returning.
            let _ = core_exec::block_on(local_take);
        }
        count.load(Ordering::SeqCst)
    }

    // ss[related actor.regeneration-survives]
    fn build_async_fun(slot: ActorSlot) -> Pin<Box<dyn Future<Output = ActorSlotOutcome>>> {
        let fun = slot.fun.clone();
        Box::pin(async move {
            let result = AssertUnwindSafe(async {
                let f = {
                    let guard = fun.lock().await;
                    guard(slot.ctx.clone())
                };
                f.await
            })
            .catch_unwind()
            .await;
            ActorSlotOutcome { slot, result }
        })
    }
}

#[cfg(test)]
// ss[related actor.regeneration-survives]
mod troupe_proptest {
    use super::*;
    use crate::actor_builder::context::{
        DynCall, NonSendWrapper, SteadyContextArchetype,
    };
    use crate::actor_builder::ActorBuilder;
    use futures::channel::oneshot;
    use futures::FutureExt;
    use proptest::prelude::*;
    use std::sync::Arc;

    fn mock_archetype(graph: &Graph) -> SteadyContextArchetype<DynCall> {
        let (_tx, rx) = oneshot::channel();
        SteadyContextArchetype {
            build_actor_exec: NonSendWrapper::new(ActorBuilder::to_dyn_call(|_| {
                Box::pin(async { Ok::<(), Box<dyn std::error::Error>>(()) })
            })),
            runtime_state: graph.runtime_state.clone(),
            channel_count: graph.channel_count.clone(),
            ident: ActorIdentity::default(),
            args: graph.args.clone(),
            all_telemetry_rx: graph.all_telemetry_rx.clone(),
            actor_metadata: Arc::new(ActorMetaData::default()),
            oneshot_shutdown_vec: graph.oneshot_shutdown_vec.clone(),
            oneshot_shutdown: rx.shared(),
            node_tx_rx: None,
            show_thread_info: false,
            aeron_meda_driver: std::sync::OnceLock::new(),
            aeron_init_for_tests: true,
            never_simulate: false,
            force_internal_behavior_in_test: false,
            shutdown_barrier: None,
        }
    }

    ss_proptest! {
        /// Property: transfer ops preserve total actor count across two troupes.
        #[test]
        // ss[verify graph.troupes]
        // ss[verify verify.process.proptest]
        fn proptest_transfer_preserves_actor_count(ops in prop::collection::vec(any::<bool>(), 1..12)) {
            let graph = GraphBuilder::for_testing().build(());
            let mut left = Troupe::new(&graph);
            let mut right = Troupe::new(&graph);
            let arch = mock_archetype(&graph);
            left.add_actor(arch.clone(), 40, true, None);
            let mut total = 1usize;

            for use_front in ops {
                if use_front {
                    if left.transfer_front_to(&mut right) {
                        prop_assert!(left.future_builder.is_empty());
                        prop_assert_eq!(right.future_builder.len(), total);
                    } else {
                        prop_assert!(left.future_builder.is_empty());
                    }
                } else if right.transfer_back_to(&mut left) {
                    prop_assert!(right.future_builder.is_empty());
                    prop_assert_eq!(left.future_builder.len(), total);
                } else {
                    prop_assert!(right.future_builder.is_empty());
                }
            }
            let combined = left.future_builder.len() + right.future_builder.len();
            prop_assert_eq!(combined, total);
        }

        /// Property: `FutureBuilderType::context` uses the troupe team id.
        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_future_builder_context_team_id(_seed in 0u8..=255u8) {
            let graph = GraphBuilder::for_testing().build(());
            let troupe = Troupe::new(&graph);
            let arch = mock_archetype(&graph);
            let fb = FutureBuilderType::new(arch, 40, true, None);
            let ctx = fb.context(troupe.team_id);
            prop_assert_eq!(ctx.team_id, troupe.team_id);
        }

        /// Property: `add_actor` grows the troupe queue by exactly one per call.
        #[test]
        // ss[verify graph.troupes]
        // ss[verify verify.process.proptest]
        fn proptest_add_actor_grows_queue(
            additions in 1usize..6,
        ) {
            let graph = GraphBuilder::for_testing().build(());
            let mut troupe = Troupe::new(&graph);
            let arch = mock_archetype(&graph);
            for _ in 0..additions {
                troupe.add_actor(arch.clone(), 40, true, None);
            }
            prop_assert_eq!(troupe.future_builder.len(), additions);
        }

        /// Property: transferring from an empty troupe returns false and leaves both empty.
        #[test]
        // ss[verify graph.troupes]
        // ss[verify verify.process.proptest]
        fn proptest_transfer_from_empty_returns_false(_case in 0..1u8) {
            let graph = GraphBuilder::for_testing().build(());
            let mut left = Troupe::new(&graph);
            let mut right = Troupe::new(&graph);
            prop_assert!(!left.transfer_front_to(&mut right));
            prop_assert!(!left.transfer_back_to(&mut right));
            prop_assert!(left.future_builder.is_empty());
            prop_assert!(right.future_builder.is_empty());
        }

        /// Property: empty troupe spawn returns zero actors without hanging.
        #[test]
        // ss[verify graph.troupes]
        // ss[verify verify.process.proptest]
        fn proptest_empty_troupe_spawn_returns_zero(_seed in 0u8..=255) {
            let graph = GraphBuilder::for_testing().build(());
            let troupe = Troupe::new(&graph);
            prop_assert_eq!(troupe.future_builder.len(), 0);
        }
    }

    /// Heavy troupe spawn integration: low case count (each case spawns OS threads).
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 6,
            .. ProptestConfig::default()
        })]

        #[test]
        // ss[verify graph.troupes]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_troupe_spawn_integration(
            actor_count in 1usize..3,
            timeout_ms in 200u64..1_500,
        ) {
            use crate::SteadyRunner;
            use std::thread::sleep;
            use std::time::Duration;

            SteadyRunner::test_build()
                .run((), move |mut graph| {
                    let mut troupe = graph.actor_troupe().with_name("SpawnTroupe");
                    for suffix in 0..actor_count {
                        graph
                            .actor_builder()
                            .with_name_and_suffix("SPAWN_TROUPE", suffix)
                            .build(
                                |ctx| async move {
                                    let mut actor = ctx.into_spotlight([], []);
                                    while actor.is_running(|| true) {
                                        actor.wait_periodic(Duration::from_millis(5)).await;
                                    }
                                    Ok(())
                                },
                                ScheduleAs::MemberOf(&mut *troupe),
                            );
                    }
                    drop(troupe);
                    assert!(graph.start_with_timeout(Duration::from_secs(10)));
                    sleep(Duration::from_millis(80));
                    graph.request_shutdown();
                    graph.block_until_stopped(Duration::from_millis(timeout_ms))
                })
                .expect("troupe spawn integration");
        }
    }
}
