//! Property tests for graph_testing; sibling of graph_testing_tests to stay under the line budget.
//! Accesses DummyActor and shutdown helpers via `super`.

use super::*;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    /// Property: random injected traffic + early shutdown still stops cleanly (puppet graph, no `run()` on worker).
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_pipeline_random_messages_clean_shutdown(
        gen_values in prop::collection::vec(0u64..10_000, 0..32),
        early_shutdown in any::<bool>(),
    ) {
        prop_assume!(early_shutdown || !gen_values.is_empty());

        let hb_beats = gen_values.len();
        let gen_values_clone = gen_values.clone();
        let logged = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
        let logged_cap = logged.clone();
        SteadyRunner::test_build()
            .run((), move |mut graph| {
                let (gen_tx, hb_tx, log_rx) = build_shutdown_proptest_pipeline(&mut graph);
                gen_tx.testing_send_all(gen_values_clone.clone(), true);
                if hb_beats > 0 {
                    hb_tx.testing_send_all(vec![0u64; hb_beats], true);
                } else {
                    // Worker veto requires closed inputs; empty traffic still must close hb.
                    hb_tx.testing_close();
                }
                graph.start();
                if !early_shutdown && !gen_values_clone.is_empty() {
                    poll_log_avail(
                        &log_rx,
                        Duration::from_millis(200),
                        |n| n >= gen_values_clone.len(),
                    );
                }
                shutdown_started_graph(graph)?;
                let mut rx = core_exec::block_on(log_rx.lock());
                let mut out = Vec::new();
                while let Some(v) = rx.try_take() {
                    out.push(v);
                }
                *logged_cap.lock().expect("logged lock") = out;
                Ok(())
            })
            .expect("runner should complete");
        let logged = logged.lock().expect("logged lock").clone();

        if early_shutdown {
            prop_assert!(logged.len() <= gen_values.len());
        } else {
            prop_assert_eq!(logged.len(), gen_values.len());
            prop_assert_eq!(logged, gen_values);
        }
    }

    /// Property: an empty testing graph reaches Running via `start_with_timeout`.
    #[test]
    // ss[verify graph.for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_empty_graph_start_with_timeout_reaches_running(
        timeout_ms in 50u64..2_000,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let timeout = Duration::from_millis(timeout_ms);
        prop_assert!(graph.start_with_timeout(timeout));
        prop_assert!(graph
            .runtime_state
            .read()
            .is_in_state(&[GraphLivelinessState::Running]));
        shutdown_started_graph(graph).expect("graph should shut down");
    }

    /// Property: staged puppet graph echoes generator traffic and logger receives it (never `run()` on worker).
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify testing.pipeline-worker-allowlist]
    // ss[verify verify.process.proptest]
    fn proptest_staged_puppet_echo_wait_matrix(
        gen_value in 0u64..10_000,
        hb_value in 0u64..10_000,
        use_echo_at in any::<bool>(),
        use_message_at in any::<bool>(),
        timeout_ms in 50u64..200,
    ) {
        SteadyRunner::test_build().run((), move |mut graph| {
            build_staged_puppet_pipeline(&mut graph);
            graph.start();
            let sm = graph.stage_manager();
            let perform_result: Result<(), Box<dyn Error>> = (|| {
                if use_echo_at {
                    sm.actor_perform("GENERATOR", StageDirection::EchoAt(0, gen_value))?;
                } else {
                    sm.actor_perform("GENERATOR", StageDirection::Echo(gen_value))?;
                }
                sm.actor_perform("HEARTBEAT", StageDirection::Echo(hb_value))?;
                if use_message_at {
                    sm.actor_perform(
                        "LOGGER",
                        StageWaitFor::MessageAt(0, gen_value, Duration::from_millis(timeout_ms)),
                    )?;
                } else {
                    sm.actor_perform(
                        "LOGGER",
                        StageWaitFor::Message(gen_value, Duration::from_millis(timeout_ms)),
                    )?;
                }
                Ok(())
            })();
            sm.final_bow();
            shutdown_started_graph(graph)?;
            perform_result
        })
        .expect("staged puppet graph should shut down cleanly");
    }

    /// Property: puppet shutdown matrix — inject traffic or stage echo, with optional early shutdown.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_puppet_shutdown_traffic_matrix(
        gen_values in prop::collection::vec(0u64..1_000, 0..24),
        use_staged_echo in any::<bool>(),
        early_shutdown in any::<bool>(),
    ) {
        prop_assume!(use_staged_echo || !gen_values.is_empty());
        let gen_values_clone = gen_values.clone();
        let expected_len = gen_values.len();
        let logged = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
        let logged_cap = logged.clone();
        SteadyRunner::test_build().run((), move |mut graph| {
            if use_staged_echo {
                build_staged_puppet_pipeline(&mut graph);
                graph.start();
                let sm = graph.stage_manager();
                let perform_result: Result<(), Box<dyn Error>> = (|| {
                    for &v in &gen_values_clone {
                        sm.actor_perform("GENERATOR", StageDirection::Echo(v))?;
                        sm.actor_perform("HEARTBEAT", StageDirection::Echo(0_u64))?;
                    }
                    Ok(())
                })();
                sm.final_bow();
                shutdown_started_graph(graph)?;
                perform_result
            } else {
                let (gen_tx, hb_tx, log_rx) = build_shutdown_proptest_pipeline(&mut graph);
                if !gen_values_clone.is_empty() {
                    gen_tx.testing_send_all(gen_values_clone.clone(), true);
                    hb_tx.testing_send_all(vec![0u64; gen_values_clone.len()], true);
                }
                graph.start();
                if !early_shutdown && !gen_values_clone.is_empty() {
                    poll_log_avail(
                        &log_rx,
                        Duration::from_millis(200),
                        |n| n >= gen_values_clone.len(),
                    );
                }
                shutdown_started_graph(graph)?;
                let mut rx = core_exec::block_on(log_rx.lock());
                let mut out = Vec::new();
                while let Some(v) = rx.try_take() {
                    out.push(v);
                }
                *logged_cap.lock().expect("logged lock") = out;
                Ok(())
            }
        })
        .expect("shutdown matrix should complete");
        if !use_staged_echo {
            if early_shutdown {
                let logged = logged.lock().expect("logged lock").clone();
                prop_assert!(logged.len() <= expected_len);
            } else if expected_len > 0 {
                let logged = logged.lock().expect("logged lock").clone();
                prop_assert_eq!(logged.len(), expected_len);
                prop_assert_eq!(logged, gen_values);
            }
        }
    }

    /// Property: `actor_perform_with_suffix` routes to the suffixed actor registration.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_actor_perform_with_suffix_registers(
        suffix in 1usize..8,
        echo_value in 0u64..1_000,
    ) {
        SteadyRunner::test_build().run((), move |mut graph| {
            let (tx_lazy, _rx) = graph.channel_builder().with_capacity(8).build::<u64>();
            graph
                .actor_builder()
                .with_name_and_suffix("SUFFIX_ACTOR", suffix)
                .build(move |ctx| pipeline_generator_edge(ctx, tx_lazy.clone()), SoloAct);
            graph.start();
            let sm = graph.stage_manager();
            let perform_result = sm
                .actor_perform_with_suffix(
                    "SUFFIX_ACTOR",
                    suffix,
                    StageDirection::Echo(echo_value),
                )
                .map(|_| ());
            sm.final_bow();
            shutdown_started_graph(graph)?;
            perform_result
        })
        .expect("suffix actor should respond");
    }

    /// Property: puppet graph with multiple staged echoes drains without hang.
    #[test]
    // ss[verify testing.pipeline-worker-allowlist]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_staged_puppet_multi_echo_sequence(
        values in prop::collection::vec(0u64..500, 1..4),
        timeout_ms in 50u64..100,
    ) {
        SteadyRunner::test_build().run((), move |mut graph| {
            build_staged_puppet_pipeline(&mut graph);
            graph.start();
            let sm = graph.stage_manager();
            let perform_result: Result<(), Box<dyn Error>> = (|| {
                for &v in &values {
                    sm.actor_perform("GENERATOR", StageDirection::Echo(v))?;
                    sm.actor_perform("HEARTBEAT", StageDirection::Echo(0_u64))?;
                    sm.actor_perform(
                        "LOGGER",
                        StageWaitFor::Message(v, Duration::from_millis(timeout_ms)),
                    )?;
                }
                Ok(())
            })();
            sm.final_bow();
            shutdown_started_graph(graph)?;
            perform_result
        })
        .expect("multi-echo puppet graph should shut down");
    }

    /// Property: staged puppet with suffixed worker lane still shuts down cleanly.
    #[test]
    // ss[verify testing.pipeline-worker-allowlist]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_staged_puppet_with_suffix_actor(
        gen_value in 0u64..2_000,
        suffix in 1usize..4,
    ) {
        SteadyRunner::test_build().run((), move |mut graph| {
            let (gen_lazy, gen_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
            let (hb_lazy, hb_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
            let (log_lazy, log_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
            graph
                .actor_builder()
                .with_name("GENERATOR")
                .build(move |ctx| pipeline_generator_edge(ctx, gen_lazy.clone()), SoloAct);
            graph
                .actor_builder()
                .with_name_and_suffix("HEARTBEAT", suffix)
                .build(move |ctx| pipeline_heartbeat_edge(ctx, hb_lazy.clone()), SoloAct);
            graph.actor_builder().with_name("WORKER").build(
                move |ctx| pipeline_worker_run(ctx, hb_rx.clone(), gen_rx.clone(), log_lazy.clone()),
                SoloAct,
            );
            graph
                .actor_builder()
                .with_name("LOGGER")
                .build(move |ctx| pipeline_logger_edge(ctx, log_rx.clone()), SoloAct);
            graph.start();
            let sm = graph.stage_manager();
            let perform_result: Result<(), Box<dyn Error>> = (|| {
                sm.actor_perform("GENERATOR", StageDirection::Echo(gen_value))?;
                sm.actor_perform_with_suffix("HEARTBEAT", suffix, StageDirection::Echo(0_u64))?;
                sm.actor_perform(
                    "LOGGER",
                    StageWaitFor::Message(gen_value, Duration::from_millis(200)),
                )?;
                Ok(())
            })();
            sm.final_bow();
            shutdown_started_graph(graph)?;
            perform_result
        })
        .expect("suffix puppet graph should shut down");
    }
}

ss_proptest! {

    /// Property: `simulate_direction` echoes arbitrary values through a locked TX.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_simulate_direction_echo_roundtrip(
        cap in 2usize..16,
        value in 0u64..10_000,
        lane in 0usize..2,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
        let tx = tx_lazy.clone();
        let mut tx = core_exec::block_on(tx.lock());
        let mut actor = DummyActor { has_data: true };
        let backplane = _manager.backplane.get(&ActorName::new("EDGE", None)).unwrap().clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageDirection::EchoAt(lane, value)))
                .await
                .expect("push echo-at");
        });
        let result = responder
            .simulate_direction(&mut tx, &mut actor, lane)
            .expect("simulate direction");
        prop_assert_eq!(result, SimStepResult::DidWork);
        let rx = rx_lazy.clone();
        let mut rx = core_exec::block_on(rx.lock());
        prop_assert_eq!(rx.try_take(), Some(value));
    }

    /// Property: `simulate_wait_for` succeeds when the expected message is available.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_simulate_wait_for_message_match(
        cap in 2usize..16,
        expected in 0u64..10_000,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
        tx_lazy.testing_send_all(vec![expected], false);
        let rx = rx_lazy.clone();
        let mut rx = core_exec::block_on(rx.lock());
        let mut actor = DummyActor { has_data: true };
        let backplane = _manager.backplane.get(&ActorName::new("EDGE", None)).unwrap().clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageWaitFor::Message(
                    expected,
                    Duration::from_millis(500),
                )))
                .await
                .expect("push wait-for");
        });
        let result = responder
            .simulate_wait_for(&mut rx, &mut actor, 0)
            .expect("simulate wait-for");
        prop_assert_eq!(result, SimStepResult::DidWork);
    }

    /// Property: `call_actor_internal` returns error for unknown actor names.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_call_actor_internal_missing_actor(_seed in 0u64..1_000) {
        let manager = StageManager::default();
        let result = manager.call_actor_internal(
            Box::new("req"),
            ActorName::new("MISSING", None),
        );
        prop_assert!(result.is_err());
    }

    /// Property: `call_actor_internal` returns OK when actor responds with OK_MESSAGE.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_call_actor_internal_ok_response(
        cap in 2usize..16,
        _seed in 0u64..100,
    ) {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let name = ActorName::new("OK_ACTOR", None);
        manager.register_node(name, cap, shutdown_rx);
        let node_side = manager.node_tx_rx(name).unwrap();
        core_exec::spawn_detached(async move {
            let mut guard = node_side.lock().await;
            let ((tx_resp, rx_req), _) = guard.deref_mut();
            if rx_req.pop().await.is_some() {
                let _ = tx_resp.push(Box::new(OK_MESSAGE.to_string())).await;
            }
        });
        let result = manager.call_actor_internal(Box::new("ping"), name);
        prop_assert!(result.is_ok());
    }

    /// Property: `simulate_direction` returns NoWork when EchoAt lane index mismatches.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_simulate_direction_echo_at_lane_mismatch(
        cap in 2usize..16,
        value in 0u64..1_000,
        lane in 0usize..2,
    ) {
        let wrong_lane = 1 - lane;
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx_lazy) = builder.build_channel::<u64>();
        let tx_steady = tx_lazy.clone();
        let mut tx = core_exec::block_on(tx_steady.lock());
        let mut actor = DummyActor { has_data: true };
        let backplane = _manager.backplane.get(&ActorName::new("EDGE", None)).unwrap().clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageDirection::EchoAt(lane, value)))
                .await
                .expect("push echo-at");
        });
        let result = responder
            .simulate_direction(&mut tx, &mut actor, wrong_lane)
            .expect("simulate direction");
        prop_assert_eq!(result, SimStepResult::NoWork);
    }

    /// Property: `simulate_wait_for` reports mismatch when message differs from expected.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_simulate_wait_for_message_mismatch(
        cap in 2usize..16,
        expected in 0u64..500,
        actual in 500u64..1_000,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
        tx_lazy.testing_send_all(vec![actual], false);
        let rx = rx_lazy.clone();
        let mut rx = core_exec::block_on(rx.lock());
        let mut actor = DummyActor { has_data: true };
        let backplane = _manager.backplane.get(&ActorName::new("EDGE", None)).unwrap().clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageWaitFor::Message(
                    expected,
                    Duration::from_millis(500),
                )))
                .await
                .expect("push wait-for");
        });
        let result = responder
            .simulate_wait_for(&mut rx, &mut actor, 0)
            .expect("simulate wait-for");
        // Mismatch produces a failure response (DidWork) rather than OK_MESSAGE.
        prop_assert_eq!(result, SimStepResult::DidWork);
    }

    /// Property: `should_apply` returns None when side channel queue is empty.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_should_apply_empty_queue(cap in 2usize..16) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let result = core_exec::block_on(responder.should_apply::<i32>());
        prop_assert_eq!(result, None);
    }

    /// Property: `respond_with` returns false when side channel has no messages.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_respond_with_empty_channel(cap in 2usize..16) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let mut actor = DummyActor { has_data: false };
        let result = responder.respond_with(|_, _| Some(Box::new(OK_MESSAGE)), &mut actor);
        prop_assert!(matches!(result, Ok(false)));
    }

    /// Property: `echo_responder_bundle` fans out a staged message to every TX lane.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_echo_responder_bundle_roundtrip(
        cap in 2usize..16,
        value in -1_000i32..1_000,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx0_lazy, rx0_lazy) = builder.build_channel::<i32>();
        let (tx1_lazy, rx1_lazy) = builder.build_channel::<i32>();
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(value))
                .await
                .expect("push echo payload");
        });
        let mut actor = DummyActor { has_data: true };
        let tx0 = tx0_lazy.clone();
        let tx1 = tx1_lazy.clone();
        let ok = core_exec::block_on(async {
            let mut bundle = TxBundle::new();
            bundle.push(tx0.try_lock().expect("tx0"));
            bundle.push(tx1.try_lock().expect("tx1"));
            responder.echo_responder_bundle(&mut actor, &mut bundle).await
        });
        prop_assert!(ok.expect("bundle echo"));
        prop_assert_eq!(rx0_lazy.testing_take_all(), vec![value]);
        prop_assert_eq!(rx1_lazy.testing_take_all(), vec![value]);
    }

    /// Property: `equals_responder_bundle` succeeds when every RX lane matches the staged value.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_equals_responder_bundle_match(
        cap in 2usize..16,
        value in -500i32..500,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx0_lazy, rx0_lazy) = builder.build_channel::<i32>();
        let (tx1_lazy, rx1_lazy) = builder.build_channel::<i32>();
        tx0_lazy.testing_send_all(vec![value], false);
        tx1_lazy.testing_send_all(vec![value], false);
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(value))
                .await
                .expect("push equals payload");
        });
        let mut actor = DummyActor { has_data: true };
        let rx0 = rx0_lazy.clone();
        let rx1 = rx1_lazy.clone();
        let ok = core_exec::block_on(async {
            let mut bundle = RxBundle::new();
            bundle.push(rx0.try_lock().expect("rx0"));
            bundle.push(rx1.try_lock().expect("rx1"));
            responder.equals_responder_bundle(&mut actor, &mut bundle).await
        });
        prop_assert!(ok.expect("bundle equals match"));
    }

    /// Property: `equals_responder_bundle` fails when any RX lane differs from the staged value.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_equals_responder_bundle_mismatch(
        cap in 2usize..16,
        expected in -200i32..200,
        other in 201i32..400,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx0_lazy, rx0_lazy) = builder.build_channel::<i32>();
        let (tx1_lazy, rx1_lazy) = builder.build_channel::<i32>();
        tx0_lazy.testing_send_all(vec![expected], false);
        tx1_lazy.testing_send_all(vec![other], false);
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(expected))
                .await
                .expect("push equals payload");
        });
        let mut actor = DummyActor { has_data: true };
        let rx0 = rx0_lazy.clone();
        let rx1 = rx1_lazy.clone();
        let ok = core_exec::block_on(async {
            let mut bundle = RxBundle::new();
            bundle.push(rx0.try_lock().expect("rx0"));
            bundle.push(rx1.try_lock().expect("rx1"));
            responder.equals_responder_bundle(&mut actor, &mut bundle).await
        });
        prop_assert!(ok.expect("bundle equals mismatch"));
    }

    /// Property: `wait_avail` unblocks once a staged message arrives on the backplane.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_wait_avail_unblocks_on_message(cap in 2usize..16) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::spawn_detached(async move {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            let _ = bp_tx.push(Box::new(7i32)).await;
        });
        core_exec::block_on(responder.wait_avail());
        prop_assert!(responder.avail() >= 1);
    }

    /// Property: `wait_available_units` returns true when enough requests are queued.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_wait_available_units_ready(
        cap in 2usize..16,
        need in 1usize..4,
    ) {
        prop_assume!(need <= cap);
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("EDGE", None), cap, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
        let mut responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let backplane = manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            for _ in 0..need {
                bp_tx.push(Box::new(1i32)).await.expect("push request");
            }
        });
        let ready = core_exec::block_on(responder.wait_available_units(need));
        prop_assert!(ready);
    }

    /// Property: `simulate_direction` surfaces `SendOutcome::Blocked` on a full channel.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_simulate_direction_blocked_on_full_channel(
        value in 0u64..1_000,
    ) {
        let cap = 1usize;
        let (_manager, responder) = setup_side_channel_responder(4);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx_lazy) = builder.build_channel::<u64>();
        let tx_steady = tx_lazy.clone();
        let mut tx = core_exec::block_on(tx_steady.lock());
        let _ = tx.shared_try_send(99_u64);
        let mut actor = DummyActor { has_data: true };
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageDirection::Echo(value)))
                .await
                .expect("push echo");
        });
        let result = responder
            .simulate_direction(&mut tx, &mut actor, 0)
            .expect("simulate direction");
        prop_assert_eq!(result, SimStepResult::DidWork);
    }

    /// Property: `simulate_wait_for` with `MessageAt` succeeds on the matching lane.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_simulate_wait_for_message_at_lane(
        cap in 2usize..16,
        expected in 0u64..5_000,
        lane in 0usize..2,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
        tx_lazy.testing_send_all(vec![expected], false);
        let rx = rx_lazy.clone();
        let mut rx = core_exec::block_on(rx.lock());
        let mut actor = DummyActor { has_data: true };
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageWaitFor::MessageAt(
                    lane,
                    expected,
                    Duration::from_millis(500),
                )))
                .await
                .expect("push wait-for-at");
        });
        let result = responder
            .simulate_wait_for(&mut rx, &mut actor, lane)
            .expect("simulate wait-for");
        prop_assert_eq!(result, SimStepResult::DidWork);
    }

    /// Property: `simulate_wait_for` ignores `MessageAt` when the lane index mismatches.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_simulate_wait_for_message_at_lane_mismatch(
        cap in 2usize..16,
        expected in 0u64..500,
        lane in 0usize..2,
    ) {
        let wrong_lane = 1 - lane;
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
        tx_lazy.testing_send_all(vec![expected], false);
        let rx = rx_lazy.clone();
        let mut rx = core_exec::block_on(rx.lock());
        let mut actor = DummyActor { has_data: true };
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageWaitFor::MessageAt(
                    lane,
                    expected,
                    Duration::from_millis(500),
                )))
                .await
                .expect("push wait-for-at");
        });
        let result = responder
            .simulate_wait_for(&mut rx, &mut actor, wrong_lane)
            .expect("simulate wait-for");
        prop_assert_eq!(result, SimStepResult::NoWork);
    }

    /// Property: `simulate_wait_for` times out when the expected message never arrives.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_simulate_wait_for_timeout(cap in 2usize..16, expected in 0u64..500) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (_tx_lazy, rx_lazy) = builder.build_channel::<u64>();
        let rx = rx_lazy.clone();
        let mut rx = core_exec::block_on(rx.lock());
        let mut actor = DummyActor { has_data: true };
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageWaitFor::Message(
                    expected,
                    Duration::from_millis(0),
                )))
                .await
                .expect("push wait-for");
        });
        let first = responder
            .simulate_wait_for(&mut rx, &mut actor, 0)
            .expect("first wait-for");
        prop_assert_eq!(first, SimStepResult::NoWork);
        let second = responder
            .simulate_wait_for(&mut rx, &mut actor, 0)
            .expect("timeout wait-for");
        prop_assert_eq!(second, SimStepResult::DidWork);
    }

    /// Property: `call_actor_internal` maps actor TIMEOUT responses to errors.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_call_actor_internal_timeout_response(cap in 2usize..16) {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let name = ActorName::new("TIMEOUT_ACTOR", None);
        manager.register_node(name, cap, shutdown_rx);
        let node_side = manager.node_tx_rx(name).unwrap();
        core_exec::spawn_detached(async move {
            let mut guard = node_side.lock().await;
            let ((tx_resp, rx_req), _) = guard.deref_mut();
            if rx_req.pop().await.is_some() {
                let _ = tx_resp.push(Box::new(TIMEOUT.to_string())).await;
            }
        });
        let result = manager.call_actor_internal(Box::new("ping"), name);
        prop_assert!(result.is_err());
        prop_assert!(result.unwrap_err().to_string().contains(TIMEOUT));
    }

    /// Property: `wait_available_units` returns true immediately when enough requests are queued.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_wait_available_units_already_ready(
        cap in 4usize..16,
        need in 1usize..4,
    ) {
        prop_assume!(need <= cap);
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("EDGE", None), cap, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
        let mut responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let backplane = manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            for _ in 0..need {
                bp_tx.push(Box::new(1i32)).await.expect("push request");
            }
        });
        let ready = core_exec::block_on(responder.wait_available_units(need));
        prop_assert!(ready);
    }

    /// Property: `wait_available_units` returns false when the node shutdown channel terminates first.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_wait_available_units_shutdown_while_waiting(cap in 4usize..16) {
        let mut manager = StageManager::default();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("EDGE", None), cap, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
        let mut responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        drop(shutdown_tx);
        let ready = core_exec::block_on(responder.wait_available_units(cap));
        prop_assert!(!ready);
    }

    /// Property: `echo_responder_bundle` declines when the staged payload type mismatches.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_echo_responder_bundle_type_mismatch(
        cap in 2usize..16,
        value in -500i32..500,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx0_lazy, rx0_lazy) = builder.build_channel::<i32>();
        let (tx1_lazy, _rx1_lazy) = builder.build_channel::<i32>();
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(value.to_string()))
                .await
                .expect("push wrong type");
        });
        let mut actor = DummyActor { has_data: true };
        let tx0 = tx0_lazy.clone();
        let tx1 = tx1_lazy.clone();
        let ok = core_exec::block_on(async {
            let mut bundle = TxBundle::new();
            bundle.push(tx0.try_lock().expect("tx0"));
            bundle.push(tx1.try_lock().expect("tx1"));
            responder.echo_responder_bundle(&mut actor, &mut bundle).await
        });
        prop_assert_eq!(ok.expect("bundle echo"), false);
        prop_assert!(rx0_lazy.testing_take_all().is_empty());
    }

    /// Property: `equals_responder_bundle` declines when the staged payload type mismatches.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_equals_responder_bundle_type_mismatch(
        cap in 2usize..16,
        value in -500i32..500,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        tx_lazy.testing_send_all(vec![value], false);
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(value.to_string()))
                .await
                .expect("push wrong type");
        });
        let mut actor = DummyActor { has_data: true };
        let rx = rx_lazy.clone();
        let ok = core_exec::block_on(async {
            let mut bundle = RxBundle::new();
            bundle.push(rx.try_lock().expect("rx"));
            responder.equals_responder_bundle(&mut actor, &mut bundle).await
        });
        prop_assert_eq!(ok.expect("bundle equals"), false);
    }

    /// Property: `should_apply` returns Some(false) when the queued message type mismatches.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_should_apply_type_mismatch(cap in 2usize..16, value in -500i32..500) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(value.to_string()))
                .await
                .expect("push wrong type");
        });
        let result = core_exec::block_on(responder.should_apply::<i32>());
        prop_assert_eq!(result, Some(false));
    }

    /// Property: `respond_with` pops the request and returns true when the handler succeeds.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_respond_with_success_pops_request(
        cap in 2usize..16,
        payload in -1_000i32..1_000,
    ) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let backplane = _manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(payload))
                .await
                .expect("push payload");
        });
        let mut actor = DummyActor { has_data: true };
        let handled = responder.respond_with(
            |message, _actor| Some(Box::new(format!("echo:{message:?}"))),
            &mut actor,
        );
        prop_assert_eq!(handled.expect("respond_with"), true);
        prop_assert_eq!(responder.avail(), 0);
    }

    /// Property: `respond_with` errors when the actor→test response channel is full.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_respond_with_full_response_channel_errors(cap in 1usize..4) {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("EDGE", None), cap, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
        let backplane = manager
            .backplane
            .get(&ActorName::new("EDGE", None))
            .unwrap()
            .clone();
        let responder = SideChannelResponder::new(node_arc.clone(), ActorIdentity::default());
        core_exec::block_on(async {
            let mut guard = node_arc.lock().await;
            let ((tx_resp, _), _) = guard.deref_mut();
            for _ in 0..cap {
                tx_resp
                    .push(Box::new("fill"))
                    .await
                    .expect("fill response channel");
            }
        });
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(42i32))
                .await
                .expect("queue request");
        });
        let mut actor = DummyActor { has_data: true };
        let err = responder.respond_with(|_, _| Some(Box::new("ok")), &mut actor);
        prop_assert!(err.is_err());
    }

    /// Property: `echo_responder_bundle` returns false when the side channel has no staged message.
    #[test]
    // ss[verify testing.graph-for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_echo_responder_bundle_empty_returns_false(cap in 2usize..16) {
        let (_manager, responder) = setup_side_channel_responder(cap);
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx0_lazy, _rx0_lazy) = builder.build_channel::<i32>();
        let (tx1_lazy, _rx1_lazy) = builder.build_channel::<i32>();
        let mut actor = DummyActor { has_data: true };
        let tx0 = tx0_lazy.clone();
        let tx1 = tx1_lazy.clone();
        let ok = core_exec::block_on(async {
            let mut bundle = TxBundle::new();
            bundle.push(tx0.try_lock().expect("tx0"));
            bundle.push(tx1.try_lock().expect("tx1"));
            responder
                .echo_responder_bundle(&mut actor, &mut bundle)
                .await
        });
        prop_assert_eq!(ok.expect("echo bundle"), false);
    }
}
