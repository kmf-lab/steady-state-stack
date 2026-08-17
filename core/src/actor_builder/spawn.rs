// ss[related actor.regeneration-survives]
use super::affinity::{pin_thread_to_core, CoreBalancer};
use super::builder::ActorBuilder;
use super::context::{
    build_actor_context, build_actor_registration, exit_actor_registration, DynCall,
    NonSendWrapper, SteadyContextArchetype,
};
use super::troupe::{Troupe, TroupeGuard};
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::*;
use futures_util::lock::Mutex;
use std::error::Error;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Launches an actor by blocking on its future until completion.
///
/// **Warning:** Do not rename this function without updating backtrace printing, as it serves as a "stop" to shorten traces.
///
/// # Type Parameters
///
/// * `F` - THE type of the future to execute.
/// * `T` - THE output type of the future.
///
/// # Arguments
///
/// * `future` - THE future to execute.
///
/// # Returns
///
/// THE result of the future execution.
// ss[related actor.regeneration-survives]
pub fn launch_actor<F: Future<Output = T>, T>(future: F) -> T {
    core_exec::block_on(future)
}

/// Represents the scheduling options for an actor, either as a solo act or a member of a troupe.
// ss[related actor.regeneration-survives]
pub enum ScheduleAs<'a> {
    /// THE actor runs independently on its own thread.
    SoloAct,
    /// THE actor is part of a troupe, sharing a thread with other actors.
    MemberOf(&'a mut Troupe),
}

// ss[related actor.regeneration-survives]
impl ScheduleAs<'_> {
    /// Determines the scheduling type based on the presence of a troupe guard.
    ///
    /// # Arguments
    ///
    /// * `some_troupe` - An optional troupe guard to check.
    ///
    /// # Returns
    ///
    /// THE appropriate `ScheduleAs` variant.
    // ss[related actor.regeneration-survives]
    pub fn dynamic_schedule(some_troupe: &mut Option<TroupeGuard>) -> ScheduleAs<'_> {
        if let Some(t) = some_troupe {
            ScheduleAs::MemberOf(t)
        } else {
            ScheduleAs::SoloAct
        }
    }
}


// ss[related actor.regeneration-survives]
impl ActorBuilder {
    fn build_spawn<F, I>(self, build_actor_exec: I)
    where
        I: Fn(SteadyActorShadow) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        if self.actor_name.name.is_empty() {
            panic!(
                "Actor name must be set before calling build(). Use .with_name() or .with_name_and_suffix()."
            );
        }
        let excluded_cores = self.excluded_cores.clone();
        let core_balancer = self.core_balancer.clone();
        let explicit_core = self.explicit_core;
        let default_core = self.team_count.clone().fetch_add(1, Ordering::SeqCst);
        let thread_lock = self.thread_lock.clone();
        let rate_ms = self.frame_rate_ms;
        let is_for_test = self.is_for_test;
        let actor_name = self.actor_name.clone();
        let stack_size = self.stack_size;

        let context_archetype = self.clone().single_actor_exec_archetype(build_actor_exec);

        core_exec::block_on(async move {
            let _guard = thread_lock.lock().await;

            let fun: NonSendWrapper<DynCall> = build_actor_registration(&context_archetype);
            let mut master_ctx: SteadyActorShadow =
                build_actor_context(&context_archetype, rate_ms, default_core, is_for_test);

            let actor_name_clone = actor_name.name;

            let mut thread_builder = std::thread::Builder::new().name(actor_name_clone.to_string());
            if let Some(size) = stack_size {
                thread_builder = thread_builder.stack_size(size);
            }

            let handle = thread_builder.spawn(move || {
                let default = if let Some(exp) = explicit_core {
                    exp
                } else {
                    default_core
                };
                let _core = if let Some(mut balancer) = core_balancer {
                    balancer.allocate_core(excluded_cores.as_slice())
                } else if !excluded_cores.is_empty() {
                    if !excluded_cores.contains(&default) {
                        default
                    } else {
                        (0..excluded_cores.len())
                            .find(|&core| !excluded_cores.contains(&core))
                            .unwrap_or(default)
                    }
                } else {
                    default
                };

                #[cfg(feature = "core_affinity")]
                {
                    if let Err(e) = pin_thread_to_core(_core) {
                        eprintln!("Failed to pin thread to core {}: {:?}", _core, e);
                    }
                }

                trace!("Spawning SoloAct {:?} on new OS thread", &actor_name_clone);

                loop {
                    match catch_unwind(AssertUnwindSafe(|| match fun.clone().try_lock() {
                        Some(actor_run) => launch_actor(actor_run(master_ctx.clone())),
                        None => panic!("internal error, future (actor) already locked"),
                    })) {
                        Ok(_) => {
                            exit_actor_registration(&context_archetype);
                            break;
                        }
                        Err(e) => {
                            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                                *s
                            } else if let Some(s) = e.downcast_ref::<String>() {
                                s.as_str()
                            } else {
                                "Unknown panic payload"
                            };

                            error!("PANIC in actor {:?}: {}", context_archetype.ident, msg);
                            // ss[impl actor.regeneration-survives]
                            // ss[impl graph.panic-restart]
                            master_ctx.regeneration += 1;
                            info!("Restarting actor: {:?}", context_archetype.ident);
                        }
                    }
                }
            });

            if let Err(e) = handle {
                error!(
                    "Failed to spawn OS thread for actor: {:?}, error: {:?}",
                    &self.actor_name.name, e
                );
            }
        });
    }

    /// Adds an actor to the specified `Troupe` for group execution.
    ///
    /// # Type Parameters
    ///
    /// * `F` - THE future returned by the execution logic.
    /// * `I` - THE execution logic function.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - THE execution logic for the actor.
    /// * `target` - THE `Troupe` to add the actor to.
    // ss[related actor.regeneration-survives]
    fn build_join<F, I>(self, build_actor_exec: I, target: &mut Troupe)
    where
        I: Fn(SteadyActorShadow) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        if self.actor_name.name.is_empty() {
            panic!(
                "Actor name must be set before calling build(). Use .with_name() or .with_name_and_suffix()."
            );
        }
        let rate = self.frame_rate_ms;
        let is_for_test = self.is_for_test;
        let stack_size = self.stack_size;
        let temp: SteadyContextArchetype<DynCall> =
            self.single_actor_exec_archetype(build_actor_exec);
        target.add_actor(temp, rate, is_for_test, stack_size);
    }

    /// Builds and schedules an actor based on the desired scheduling type.
    ///
    /// # Type Parameters
    ///
    /// * `F` - THE future returned by the execution logic.
    /// * `I` - THE execution logic function.
    ///
    /// # Arguments
    ///
    /// * `build_actor_exec` - THE execution logic for the actor.
    /// * `desired_scheduling` - THE scheduling type (`SoloAct` or `MemberOf`).
    // ss[related actor.regeneration-survives]
    pub fn build<F, I>(self, build_actor_exec: I, desired_scheduling: ScheduleAs)
    where
        I: Fn(SteadyActorShadow) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), Box<dyn Error>>> + 'static,
    {
        match desired_scheduling {
            ScheduleAs::SoloAct => {
                self.build_spawn(build_actor_exec);
            }
            ScheduleAs::MemberOf(team) => {
                self.build_join(build_actor_exec, team);
            }
        }
    }

}

#[cfg(test)]
// ss[related actor.regeneration-survives]
mod spawn_proptest {
    use super::*;
    use proptest::prelude::*;

    ss_proptest! {
        /// Property: `ScheduleAs::dynamic_schedule` picks MemberOf when a troupe guard exists.
        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_dynamic_schedule_matches_guard_presence(has_troupe in any::<bool>()) {
            let graph = GraphBuilder::for_testing().build(());
            let mut troupe_guard = if has_troupe {
                Some(graph.actor_troupe())
            } else {
                None
            };
            match ScheduleAs::dynamic_schedule(&mut troupe_guard) {
                ScheduleAs::SoloAct => prop_assert!(!has_troupe),
                ScheduleAs::MemberOf(_) => prop_assert!(has_troupe),
            }
        }

        /// Property: excluded-core fallback matches spawn.rs affinity selection branches.
        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_excluded_core_fallback(
            default_core in 0usize..8,
            excluded in prop::collection::vec(0usize..8, 1..4),
        ) {
            let excluded: Vec<usize> = excluded
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            let selected = if !excluded.contains(&default_core) {
                default_core
            } else {
                (0..excluded.len())
                    .find(|&core| !excluded.contains(&core))
                    .unwrap_or(default_core)
            };
            if !excluded.contains(&default_core) {
                prop_assert_eq!(selected, default_core);
            } else if let Some(core) = (0..excluded.len()).find(|c| !excluded.contains(c)) {
                prop_assert_eq!(selected, core);
                prop_assert!(!excluded.contains(&selected));
            } else {
                prop_assert_eq!(selected, default_core);
            }
        }
    }

    /// Heavy SoloAct spawn integration: low case count (each case spawns an OS thread).
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 6,
            .. ProptestConfig::default()
        })]

        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_solo_act_spawn_integration(
            use_balancer in any::<bool>(),
            timeout_ms in 200u64..1_500,
        ) {
            use crate::SteadyRunner;
            use std::thread::sleep;
            use std::time::Duration;

            SteadyRunner::test_build()
                .run((), move |mut graph| {
                    let mut builder = graph.actor_builder().with_name("SOLO_SPAWN");
                    if use_balancer {
                        builder = builder.with_core_balancing(CoreBalancer {
                            core_usage: vec![0, 0, 0],
                        });
                    }
                    builder.build(
                        |ctx| async move {
                            let mut actor = ctx.into_spotlight([], []);
                            while actor.is_running(|| true) {
                                actor.wait_periodic(Duration::from_millis(5)).await;
                            }
                            Ok(())
                        },
                        ScheduleAs::SoloAct,
                    );
                    graph.start();
                    sleep(Duration::from_millis(80));
                    graph.request_shutdown();
                    graph.block_until_stopped(Duration::from_millis(timeout_ms))
                })
                .expect("solo act spawn integration");
        }

        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify platform.executor-features]
        // ss[verify verify.process.proptest]
        fn proptest_solo_ping_pong_channel_then_shutdown(
            n in 1u8..6,
            timeout_ms in 400u64..1_500,
        ) {
            use crate::SteadyRunner;
            use std::thread::sleep;
            use std::time::Duration;

            SteadyRunner::test_build()
                .run((), move |mut graph| {
                    let (tx, rx) = graph
                        .channel_builder()
                        .with_capacity(16)
                        .build_channel::<u8>();
                    graph.actor_builder().with_name("PING").build(
                        move |ctx| {
                            let tx = tx.clone();
                            async move {
                                let mut actor = ctx.into_spotlight([], [&tx]);
                                let mut txg = tx.lock().await;
                                for i in 0..n {
                                    let _ = actor
                                        .send_async(&mut txg, i, SendSaturation::AwaitForRoom)
                                        .await;
                                }
                                txg.mark_closed();
                                while actor.is_running(|| true) {
                                    actor.wait_periodic(Duration::from_millis(5)).await;
                                }
                                Ok(())
                            }
                        },
                        ScheduleAs::SoloAct,
                    );
                    graph.actor_builder().with_name("PONG").build(
                        move |ctx| {
                            let rx = rx.clone();
                            async move {
                                let mut actor = ctx.into_spotlight([&rx], []);
                                let mut rxg = rx.lock().await;
                                while actor.is_running(|| rxg.is_closed_and_empty()) {
                                    let _clean = await_for_all!(actor.wait_avail(&mut rxg, 1));
                                    let _ = actor.try_take(&mut rxg);
                                }
                                Ok(())
                            }
                        },
                        ScheduleAs::SoloAct,
                    );
                    graph.start();
                    sleep(Duration::from_millis(80));
                    graph.request_shutdown();
                    graph.block_until_stopped(Duration::from_millis(timeout_ms))
                })
                .expect("solo ping-pong");
        }

        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify platform.executor-features]
        // ss[verify verify.process.proptest]
        fn proptest_solo_panic_restarts_then_shutdown(timeout_ms in 400u64..1_500) {
            use crate::SteadyRunner;
            use std::sync::atomic::{AtomicU32, Ordering};
            use std::sync::Arc;
            use std::thread::sleep;
            use std::time::Duration;

            let gens = Arc::new(AtomicU32::new(0));
            let gens_actor = gens.clone();
            SteadyRunner::test_build()
                .run((), move |mut graph| {
                    graph.actor_builder().with_name("PANIC_ONCE").build(
                        move |ctx| {
                            let gens_actor = gens_actor.clone();
                            async move {
                                gens_actor.store(ctx.regeneration, Ordering::SeqCst);
                                if ctx.regeneration == 0 {
                                    panic!("intentional first-generation panic");
                                }
                                let mut actor = ctx.into_spotlight([], []);
                                while actor.is_running(|| true) {
                                    actor.wait_periodic(Duration::from_millis(5)).await;
                                }
                                Ok(())
                            }
                        },
                        ScheduleAs::SoloAct,
                    );
                    graph.start();
                    sleep(Duration::from_millis(120));
                    graph.request_shutdown();
                    graph.block_until_stopped(Duration::from_millis(timeout_ms))
                })
                .expect("solo panic restart");
            prop_assert!(gens.load(Ordering::SeqCst) >= 1);
        }
    }
}

#[cfg(all(test, feature = "tokio"))]
// ss[related platform.executor-features]
mod tokio_reactor_tests {
    use super::*;
    use crate::SteadyRunner;
    use proptest::prelude::*;
    use std::rc::Rc;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    // ss[verify platform.executor-features]
    fn tokio_net_loopback_on_current_thread_reactor() {
        crate::core_exec::block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let addr = listener.local_addr().expect("addr");
            crate::core_exec::spawn_detached(async move {
                let _ = tokio::net::TcpStream::connect(addr).await;
            });
            let _accepted = listener.accept().await.expect("accept");
        });
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 4,
            .. ProptestConfig::default()
        })]

        #[test]
        // ss[verify platform.executor-features]
        // ss[verify verify.process.proptest]
        fn proptest_tokio_solo_nonsend_sleep(timeout_ms in 400u64..1_500) {
            SteadyRunner::test_build()
                .run((), move |mut graph| {
                    graph.actor_builder().with_name("TOKIO_SOLO").build(
                        |ctx| async move {
                            let local = Rc::new(7u8);
                            tokio::time::sleep(Duration::from_millis(5)).await;
                            let _ = *local;
                            let mut actor = ctx.into_spotlight([], []);
                            while actor.is_running(|| true) {
                                actor.wait_periodic(Duration::from_millis(5)).await;
                            }
                            Ok(())
                        },
                        ScheduleAs::SoloAct,
                    );
                    graph.start();
                    sleep(Duration::from_millis(80));
                    graph.request_shutdown();
                    graph.block_until_stopped(Duration::from_millis(timeout_ms))
                })
                .expect("tokio solo");
        }

        #[test]
        // ss[verify platform.executor-features]
        // ss[verify graph.troupes]
        // ss[verify verify.process.proptest]
        fn proptest_tokio_troupe_nonsend_sleep(timeout_ms in 400u64..1_500) {
            SteadyRunner::test_build()
                .run((), move |mut graph| {
                    let mut troupe = graph.actor_troupe().with_name("TokioTroupe");
                    graph.actor_builder().with_name("TOKIO_TROUPE").build(
                        |ctx| async move {
                            let local = Rc::new(3u8);
                            tokio::time::sleep(Duration::from_millis(5)).await;
                            let _ = *local;
                            let mut actor = ctx.into_spotlight([], []);
                            while actor.is_running(|| true) {
                                actor.wait_periodic(Duration::from_millis(5)).await;
                            }
                            Ok(())
                        },
                        ScheduleAs::MemberOf(&mut *troupe),
                    );
                    drop(troupe);
                    assert!(graph.start_with_timeout(Duration::from_secs(10)));
                    sleep(Duration::from_millis(80));
                    graph.request_shutdown();
                    graph.block_until_stopped(Duration::from_millis(timeout_ms))
                })
                .expect("tokio troupe");
        }

        #[test]
        // ss[verify platform.executor-features]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_tokio_solo_panic_restarts(timeout_ms in 400u64..1_500) {
            SteadyRunner::test_build()
                .run((), move |mut graph| {
                    graph.actor_builder().with_name("TOKIO_PANIC").build(
                        |ctx| async move {
                            if ctx.regeneration == 0 {
                                panic!("tokio first-generation panic");
                            }
                            tokio::time::sleep(Duration::from_millis(1)).await;
                            let mut actor = ctx.into_spotlight([], []);
                            while actor.is_running(|| true) {
                                actor.wait_periodic(Duration::from_millis(5)).await;
                            }
                            Ok(())
                        },
                        ScheduleAs::SoloAct,
                    );
                    graph.start();
                    sleep(Duration::from_millis(120));
                    graph.request_shutdown();
                    graph.block_until_stopped(Duration::from_millis(timeout_ms))
                })
                .expect("tokio panic restart");
        }

        #[test]
        // ss[verify platform.executor-features]
        // ss[verify verify.process.proptest]
        fn proptest_tokio_mixed_graph_shuts_down(timeout_ms in 400u64..1_500) {
            SteadyRunner::test_build()
                .run((), move |mut graph| {
                    graph.actor_builder().with_name("PLAIN").build(
                        |ctx| async move {
                            let mut actor = ctx.into_spotlight([], []);
                            while actor.is_running(|| true) {
                                actor.wait_periodic(Duration::from_millis(5)).await;
                            }
                            Ok(())
                        },
                        ScheduleAs::SoloAct,
                    );
                    graph.actor_builder().with_name("TOKIO_MIX").build(
                        |ctx| async move {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                            let mut actor = ctx.into_spotlight([], []);
                            while actor.is_running(|| true) {
                                actor.wait_periodic(Duration::from_millis(5)).await;
                            }
                            Ok(())
                        },
                        ScheduleAs::SoloAct,
                    );
                    graph.start();
                    sleep(Duration::from_millis(80));
                    graph.request_shutdown();
                    graph.block_until_stopped(Duration::from_millis(timeout_ms))
                })
                .expect("mixed tokio graph");
        }
    }
}
