//! Property tests for simulation runners and `simulated_behavior`.

// ss[related testing.sim-producer-close]
use super::*;
// ss[related philosophy.structural-hierarchy]
use async_ringbuf::producer::AsyncProducer;
// ss[related philosophy.structural-hierarchy]
use crate::channel_builder::ChannelBuilder;
// ss[related testing.sim-producer-close]
use crate::core_exec;
// ss[related philosophy.structural-hierarchy]
use crate::graph_testing::{SideChannelResponder, StageDirection, StageManager};
// ss[related philosophy.structural-hierarchy]
use crate::proptest_support::{capacity, message_vec};
// ss[related testing.sim-producer-close]
use crate::SteadyRxBundle;
// ss[related philosophy.structural-hierarchy]
use crate::SteadyTxBundle;
// ss[related philosophy.structural-hierarchy]
use crate::{ActorIdentity, ActorName, SteadyActor};
// ss[related testing.sim-producer-close]
use futures::channel::oneshot;
// ss[related philosophy.structural-hierarchy]
use proptest::prelude::*;
// ss[related philosophy.structural-hierarchy]
use std::ops::DerefMut;
// ss[related testing.sim-producer-close]
use std::sync::Arc;

// ss[related philosophy.structural-hierarchy]
struct ErrorOnStepRunner;
// ss[related testing.sim-producer-close]
struct ErrorOnStageRunner;

// ss[related philosophy.structural-hierarchy]
impl<C: SteadyActor> SimRunner<C> for ErrorOnStepRunner {
    // ss[related testing.sim-producer-close]
    fn step(&mut self) -> Result<SimStepResult, Box<dyn std::error::Error>> {
        Err("injected step error".into())
    }
}

// ss[related testing.sim-producer-close]
impl<C: SteadyActor> SimRunner<C> for ErrorOnStageRunner {
    // ss[related philosophy.structural-hierarchy]
    fn step(&mut self) -> Result<SimStepResult, Box<dyn std::error::Error>> {
        Ok(SimStepResult::NoWork)
    }

    // ss[related testing.sim-producer-close]
    fn stage_step(
        &mut self,
        _actor: &mut C,
        _responder: &SideChannelResponder,
    ) -> Result<SimStepResult, Box<dyn std::error::Error>> {
        Err("injected stage error".into())
    }
}

// ss[related testing.sim-producer-close]
struct ErrorOnStep;
// ss[related philosophy.structural-hierarchy]
impl<C: SteadyActor> IntoSimRunner<C> for ErrorOnStep {
    // ss[related philosophy.structural-hierarchy]
    fn into_sim_runner(&self) -> Box<dyn SimRunner<C>> {
        Box::new(ErrorOnStepRunner)
    }
}

// ss[related testing.sim-producer-close]
struct ErrorOnStage;
// ss[related philosophy.structural-hierarchy]
impl<C: SteadyActor> IntoSimRunner<C> for ErrorOnStage {
    // ss[related philosophy.structural-hierarchy]
    fn into_sim_runner(&self) -> Box<dyn SimRunner<C>> {
        Box::new(ErrorOnStageRunner)
    }
}

ss_proptest! {

    /// Property: `SimRx::step` drains every injected message then returns `NoWork`.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_rx_step_drains_fifo(
        cap in capacity(),
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let builder = ChannelBuilder::default().with_capacity(cap.max(1));
        let (tx, rx_lazy) = builder.build_channel::<i32>();
        if !messages.is_empty() {
            tx.testing_send_all(messages.clone(), false);
        }
        let rx = rx_lazy.clone();
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&rx);
        let mut took = 0usize;
        loop {
            match runner.step().unwrap() {
                SimStepResult::DidWork => took += 1,
                SimStepResult::NoWork => break,
            }
        }
        prop_assert_eq!(took, messages.len());
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::NoWork);
    }

    /// Property: `SimTx::step` fills the channel then returns `NoWork`.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_tx_step_fills_channel(cap in capacity()) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx) = builder.build_channel::<i32>();
        let tx = tx_lazy.clone();
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&tx);
        let mut sent = 0usize;
        loop {
            match runner.step().unwrap() {
                SimStepResult::DidWork => sent += 1,
                SimStepResult::NoWork => break,
            }
        }
        prop_assert_eq!(sent, cap);
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::NoWork);
    }

    /// Property: `SimTx::close_outputs_on_simulated_stop` marks the downstream RX closed.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_tx_close_outputs_marks_closed(cap in capacity()) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        let tx = tx_lazy.clone();
        let rx = rx_lazy.clone();
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&tx);
        runner.close_outputs_on_simulated_stop().unwrap();
        let closed = core_exec::block_on(async {
            let mut g = rx.lock().await;
            g.is_closed_and_empty()
        });
        prop_assert!(closed);
    }

    /// Property: `SimRxBundle::step` round-robins across lanes until all are empty.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_rx_bundle_round_robin(
        cap in 2usize..16,
        lane0 in message_vec::<i32>(),
        lane1 in message_vec::<i32>(),
    ) {
        let lane0: Vec<i32> = lane0.into_iter().take(cap).collect();
        let lane1: Vec<i32> = lane1.into_iter().take(cap).collect();
        let b0 = ChannelBuilder::default().with_capacity(cap);
        let (tx0, rx0_lazy) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(cap);
        let (tx1, rx1_lazy) = b1.build_channel::<i32>();
        if !lane0.is_empty() {
            tx0.testing_send_all(lane0.clone(), false);
        }
        if !lane1.is_empty() {
            tx1.testing_send_all(lane1.clone(), false);
        }
        let bundle: SteadyRxBundle<i32, 2> =
            Arc::new([rx0_lazy.clone(), rx1_lazy.clone()]);
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&bundle);
        let total = lane0.len() + lane1.len();
        let mut took = 0usize;
        for _ in 0..total.saturating_mul(4).max(1) {
            if runner.step().unwrap() == SimStepResult::DidWork {
                took += 1;
            }
            if took >= total {
                break;
            }
        }
        prop_assert_eq!(took, total);
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::NoWork);
    }

    /// Property: `SimTxBundle::close_outputs_on_simulated_stop` closes every lane.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_tx_bundle_close_outputs(cap in capacity()) {
        let b0 = ChannelBuilder::default().with_capacity(cap);
        let (tx0_lazy, rx0_lazy) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(cap);
        let (tx1_lazy, rx1_lazy) = b1.build_channel::<i32>();
        let bundle: SteadyTxBundle<i32, 2> =
            Arc::new([tx0_lazy.clone(), tx1_lazy.clone()]);
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&bundle);
        runner.close_outputs_on_simulated_stop().unwrap();
        let rx0 = rx0_lazy.clone();
        let rx1 = rx1_lazy.clone();
        let closed = core_exec::block_on(async {
            let mut g0 = rx0.lock().await;
            let mut g1 = rx1.lock().await;
            g0.is_closed_and_empty() && g1.is_closed_and_empty()
        });
        prop_assert!(closed);
    }

    /// Property: `SimStreamTx::step` sends control + payload until full.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_stream_tx_step(cap in 2usize..16) {
        // ss[related philosophy.structural-hierarchy]
        use crate::distributed::aqueduct_stream::StreamIngress;
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx) = builder.build_stream::<StreamIngress>(8);
        let tx = tx_lazy.clone();
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&tx);
        let mut sent = 0usize;
        loop {
            match runner.step().unwrap() {
                SimStepResult::DidWork => sent += 1,
                SimStepResult::NoWork => break,
            }
        }
        prop_assert!(sent > 0);
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::NoWork);
    }

    /// Property: `SimStreamRx::step` drains control messages.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_stream_rx_step(cap in 2usize..16) {
        // ss[related philosophy.structural-hierarchy]
        use crate::distributed::aqueduct_stream::StreamIngress;
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_stream::<StreamIngress>(8);
        let tx = tx_lazy.clone();
        let rx = rx_lazy.clone();
        let mut runner_tx: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&tx);
        let mut runner_rx: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&rx);
        let mut produced = 0usize;
        while runner_tx.step().unwrap() == SimStepResult::DidWork {
            produced += 1;
        }
        let mut consumed = 0usize;
        while runner_rx.step().unwrap() == SimStepResult::DidWork {
            consumed += 1;
        }
        prop_assert_eq!(consumed, produced);
    }

    /// Property: `simulated_behavior` always calls `close_outputs_on_simulated_stop` on exit.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_simulated_behavior_closes_outputs_on_stop(cap in 2usize..16) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        let tx = tx_lazy.clone();
        let rx = rx_lazy.clone();
        let mut actor = TestActor::new();
        actor.is_running = false;
        core_exec::block_on(simulated_behavior(&mut actor, vec![&tx]))
            .expect("simulated_behavior should complete");
        let rx_steady = rx.clone();
        let closed = core_exec::block_on(async {
            let mut guard = rx_steady.lock().await;
            guard.is_closed()
        });
        prop_assert!(closed);
    }

    /// Property: `SimTx::stage_step` echoes `StageDirection::Echo` through a locked TX.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_sim_tx_stage_step_echo(
        cap in 2usize..16,
        value in -1000i32..1000,
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        let tx = tx_lazy.clone();
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("SIM_TX", None), 8, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("SIM_TX", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc.clone(), ActorIdentity::default());
        let backplane = manager
            .backplane
            .get(&ActorName::new("SIM_TX", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageDirection::Echo(value)))
                .await
                .expect("push stage direction");
        });
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&tx);
        let mut actor = TestActor::new();
        let result = runner
            .stage_step(&mut actor, &responder)
            .expect("stage_step should succeed");
        prop_assert_eq!(result, SimStepResult::DidWork);
        let taken = rx_lazy.testing_take_all();
        prop_assert_eq!(taken.first().copied(), Some(value));
    }

    /// Property: `SimStreamTx::close_outputs_on_simulated_stop` marks stream RX closed.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_stream_tx_close_outputs(cap in 2usize..16) {
        // ss[related philosophy.structural-hierarchy]
        use crate::distributed::aqueduct_stream::StreamIngress;
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_stream::<StreamIngress>(8);
        let tx = tx_lazy.clone();
        let rx = rx_lazy.clone();
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&tx);
        runner.close_outputs_on_simulated_stop().unwrap();
        let closed = core_exec::block_on(async {
            let mut guard = rx.lock().await;
            guard.is_closed()
        });
        prop_assert!(closed);
    }

    /// Property: `SimRx::stage_step` waits for `StageWaitFor::Message` when data is present.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_sim_rx_stage_step_wait_for(
        cap in 2usize..16,
        expected in -500i32..500,
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        tx_lazy.testing_send_all(vec![expected], false);
        let rx = rx_lazy.clone();
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("SIM_RX", None), 8, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("SIM_RX", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let backplane = manager
            .backplane
            .get(&ActorName::new("SIM_RX", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(crate::graph_testing::StageWaitFor::Message(
                    expected,
                    std::time::Duration::from_millis(500),
                )))
                .await
                .expect("push wait-for");
        });
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&rx);
        let mut actor = TestActor::new();
        let result = runner
            .stage_step(&mut actor, &responder)
            .expect("stage_step");
        prop_assert_eq!(result, SimStepResult::DidWork);
    }

    /// Property: `simulated_behavior` with side channel drives `stage_step` until actor stops.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_simulated_behavior_stage_mode_closes_tx(
        cap in 2usize..16,
        value in -200i32..200,
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        let tx = tx_lazy.clone();
        let rx = rx_lazy.clone();
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("SIM_EDGE", None), 8, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("SIM_EDGE", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let backplane = manager
            .backplane
            .get(&ActorName::new("SIM_EDGE", None))
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
        let mut actor = TestActor {
            sidechannel: Some(responder),
            is_running: false,
        };
        core_exec::block_on(simulated_behavior(&mut actor, vec![&tx]))
            .expect("simulated_behavior");
        let closed = core_exec::block_on(async {
            let mut guard = rx.lock().await;
            guard.is_closed()
        });
        prop_assert!(closed);
    }

    /// Property: `SimTxBundle::stage_step` echoes on the matching lane index.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_sim_tx_bundle_stage_step_lane(
        cap in 2usize..16,
        lane in 0usize..2,
        value in -300i32..300,
    ) {
        let b0 = ChannelBuilder::default().with_capacity(cap);
        let (tx0_lazy, rx0_lazy) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(cap);
        let (tx1_lazy, rx1_lazy) = b1.build_channel::<i32>();
        let bundle: SteadyTxBundle<i32, 2> =
            Arc::new([tx0_lazy.clone(), tx1_lazy.clone()]);
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("SIM_BUNDLE", None), 8, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("SIM_BUNDLE", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let backplane = manager
            .backplane
            .get(&ActorName::new("SIM_BUNDLE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageDirection::EchoAt(lane, value)))
                .await
                .expect("push echo-at");
        });
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&bundle);
        let mut actor = TestActor::new();
        let result = runner
            .stage_step(&mut actor, &responder)
            .expect("stage_step");
        prop_assert_eq!(result, SimStepResult::DidWork);
        let taken = if lane == 0 {
            rx0_lazy.testing_take_all()
        } else {
            rx1_lazy.testing_take_all()
        };
        prop_assert_eq!(taken.first().copied(), Some(value));
    }

    /// Property: `simulated_behavior` propagates `step` errors from runners.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_simulated_behavior_step_error_propagates(_seed in 0u8..4) {
        let mut actor = TestActor::new();
        let err = core_exec::block_on(simulated_behavior(&mut actor, vec![&ErrorOnStep]));
        prop_assert!(err.is_err());
    }

    /// Property: `simulated_behavior` propagates `stage_step` errors when a side channel is attached.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_simulated_behavior_stage_error_propagates(cap in 2usize..16) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx) = builder.build_channel::<i32>();
        let tx = tx_lazy.clone();
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("ERR_EDGE", None), 8, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("ERR_EDGE", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let mut actor = TestActor {
            sidechannel: Some(responder),
            is_running: true,
        };
        let err = core_exec::block_on(simulated_behavior(&mut actor, vec![&tx, &ErrorOnStage]));
        prop_assert!(err.is_err());
    }

    /// Property: `SimTx::step` returns `NoWork` once the channel is full.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_tx_step_no_work_when_full(cap in 1usize..8) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx) = builder.build_channel::<i32>();
        let tx = tx_lazy.clone();
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&tx);
        let mut filled = 0usize;
        while runner.step().unwrap() == SimStepResult::DidWork {
            filled += 1;
        }
        prop_assert_eq!(filled, cap);
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::NoWork);
    }

    /// Property: `SimRxBundle::stage_step` honors `StageWaitFor::MessageAt` on the matching lane.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_sim_rx_bundle_stage_step_message_at(
        cap in 2usize..16,
        lane in 0usize..2,
        expected in -400i32..400,
    ) {
        let b0 = ChannelBuilder::default().with_capacity(cap);
        let (tx0, rx0_lazy) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(cap);
        let (tx1, rx1_lazy) = b1.build_channel::<i32>();
        if lane == 0 {
            tx0.testing_send_all(vec![expected], false);
        } else {
            tx1.testing_send_all(vec![expected], false);
        }
        let bundle: SteadyRxBundle<i32, 2> = Arc::new([rx0_lazy.clone(), rx1_lazy.clone()]);
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("SIM_RX_BUNDLE", None), 8, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("SIM_RX_BUNDLE", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let backplane = manager
            .backplane
            .get(&ActorName::new("SIM_RX_BUNDLE", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(crate::graph_testing::StageWaitFor::MessageAt(
                    lane,
                    expected,
                    std::time::Duration::from_millis(500),
                )))
                .await
                .expect("push wait-for-at");
        });
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&bundle);
        let mut actor = TestActor::new();
        let result = runner
            .stage_step(&mut actor, &responder)
            .expect("stage_step");
        prop_assert_eq!(result, SimStepResult::DidWork);
    }

    /// Property: `SimTxBundle::stage_step` ignores `EchoAt` when no bundle lane matches the index.
    #[test]
    // ss[verify testing.stage-manager-integration]
    // ss[verify verify.process.proptest]
    fn proptest_sim_tx_bundle_stage_step_lane_mismatch(
        cap in 2usize..16,
        value in -200i32..200,
    ) {
        let out_of_band_lane = 9usize;
        let b0 = ChannelBuilder::default().with_capacity(cap);
        let (tx0_lazy, rx0_lazy) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(cap);
        let (tx1_lazy, rx1_lazy) = b1.build_channel::<i32>();
        let bundle: SteadyTxBundle<i32, 2> =
            Arc::new([tx0_lazy.clone(), tx1_lazy.clone()]);
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("SIM_TX_MISMATCH", None), 8, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("SIM_TX_MISMATCH", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let backplane = manager
            .backplane
            .get(&ActorName::new("SIM_TX_MISMATCH", None))
            .unwrap()
            .clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (bp_tx, _) = guard.deref_mut();
            bp_tx
                .push(Box::new(StageDirection::EchoAt(out_of_band_lane, value)))
                .await
                .expect("push echo-at");
        });
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&bundle);
        let mut actor = TestActor::new();
        let result = runner
            .stage_step(&mut actor, &responder)
            .expect("stage_step");
        prop_assert_eq!(result, SimStepResult::NoWork);
        prop_assert!(rx0_lazy.testing_take_all().is_empty());
        prop_assert!(rx1_lazy.testing_take_all().is_empty());
    }

    /// Property: `SimStreamRx::step` returns `NoWork` on an empty control channel.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_stream_rx_step_empty(cap in 2usize..16) {
        // ss[related philosophy.structural-hierarchy]
        use crate::distributed::aqueduct_stream::StreamIngress;
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (_tx_lazy, rx_lazy) = builder.build_stream::<StreamIngress>(8);
        let rx = rx_lazy.clone();
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&rx);
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::NoWork);
    }

    /// Property: `SimTxBundle::step` skips a full lane and continues round-robin on the next.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_tx_bundle_step_skips_full_lane(cap in 2usize..16) {
        let b0 = ChannelBuilder::default().with_capacity(1);
        let (tx0_lazy, rx0_lazy) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(cap);
        let (tx1_lazy, rx1_lazy) = b1.build_channel::<i32>();
        let tx0 = tx0_lazy.clone();
        if let Some(mut g) = tx0.try_lock() {
            let _ = g.shared_try_send(0i32);
        }
        let bundle: SteadyTxBundle<i32, 2> =
            Arc::new([tx0_lazy.clone(), tx1_lazy.clone()]);
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&bundle);
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::NoWork);
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::DidWork);
        prop_assert_eq!(rx1_lazy.testing_take_all().len(), 1);
        prop_assert_eq!(rx0_lazy.testing_take_all(), vec![0i32]);
    }

    /// Property: `SimRx::close_outputs_on_simulated_stop` is a no-op (receive-side default).
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_rx_close_outputs_is_noop(cap in 2usize..16) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        tx_lazy.testing_send_all(vec![1, 2], false);
        let rx = rx_lazy.clone();
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&rx);
        runner.close_outputs_on_simulated_stop().expect("noop close");
        let still_open = core_exec::block_on(async {
            let mut guard = rx.lock().await;
            !guard.is_closed()
        });
        prop_assert!(still_open);
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::DidWork);
    }

    /// Property: `simulated_behavior` with only `SimRx` exits cleanly without closing inputs.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_simulated_behavior_sim_rx_only_exits_without_closing(
        cap in 2usize..16,
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let builder = ChannelBuilder::default().with_capacity(cap.max(1));
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        if !messages.is_empty() {
            tx_lazy.testing_send_all(messages.clone(), false);
        }
        let rx = rx_lazy.clone();
        let mut actor = TestActor::new();
        actor.is_running = false;
        core_exec::block_on(simulated_behavior(&mut actor, vec![&rx]))
            .expect("simulated_behavior should complete");
        let remaining = rx_lazy.testing_take_all();
        prop_assert_eq!(remaining, messages);
        let still_open = core_exec::block_on(async {
            let mut guard = rx.lock().await;
            !guard.is_closed()
        });
        prop_assert!(still_open);
    }

    /// Property: `never_simulate(true)` edge actors skip `simulated_behavior` and exit without staging.
    #[test]
    // ss[verify testing.never-run-in-unit]
    // ss[verify graph.for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_never_simulate_edge_skips_simulated_behavior(timeout_ms in 50u64..1_000) {
        // ss[related philosophy.structural-hierarchy]
        use crate::graph::GraphBuilder;
        // ss[related philosophy.structural-hierarchy]
        use crate::ScheduleAs;
        // ss[related testing.sim-producer-close]
        use crate::SteadyActorShadow;
        let mut graph = GraphBuilder::for_testing().build(());
        graph
            .actor_builder()
            .with_name("NEVER_SIM_EDGE")
            .never_simulate(true)
            .build(
                |ctx: SteadyActorShadow| async move {
                    let mut actor = ctx.into_spotlight([], []);
                    while actor.is_running(|| true) {}
                    Ok(())
                },
                ScheduleAs::SoloAct,
            );
        graph.start();
        graph.request_shutdown();
        let result = graph.block_until_stopped(std::time::Duration::from_millis(timeout_ms));
        prop_assert!(result.is_ok());
    }

    /// Property: `SimRxBundle::close_outputs_on_simulated_stop` is a no-op (receive-side default).
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_sim_rx_bundle_close_outputs_is_noop(cap in 2usize..16) {
        let b0 = ChannelBuilder::default().with_capacity(cap);
        let (tx0, rx0_lazy) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(cap);
        let (tx1, rx1_lazy) = b1.build_channel::<i32>();
        tx0.testing_send_all(vec![1], false);
        tx1.testing_send_all(vec![2], false);
        let bundle: SteadyRxBundle<i32, 2> = Arc::new([rx0_lazy.clone(), rx1_lazy.clone()]);
        let mut runner: Box<dyn SimRunner<TestActor>> =
            IntoSimRunner::<TestActor>::into_sim_runner(&bundle);
        runner.close_outputs_on_simulated_stop().expect("noop close");
        let rx0 = rx0_lazy.clone();
        let still_open = core_exec::block_on(async {
            let mut g0 = rx0.lock().await;
            !g0.is_closed()
        });
        prop_assert!(still_open);
        prop_assert_eq!(runner.step().unwrap(), SimStepResult::DidWork);
    }

    /// Property: mixed SimTx/SimRx simulated stop closes TX outputs but leaves RX inputs open.
    #[test]
    // ss[verify testing.sim-producer-close]
    // ss[verify verify.process.proptest]
    fn proptest_simulated_behavior_mixed_runners_close_tx_only(cap in 2usize..16) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_from_tx) = builder.build_channel::<i32>();
        let (feed_tx, rx_lazy) = builder.build_channel::<i32>();
        feed_tx.testing_send_all(vec![7], false);
        let tx = tx_lazy.clone();
        let rx = rx_lazy.clone();
        let mut actor = TestActor::new();
        actor.is_running = false;
        core_exec::block_on(simulated_behavior(&mut actor, vec![&tx, &rx]))
            .expect("simulated_behavior");
        let rx_from_tx_steady = rx_from_tx.clone();
        let tx_closed = core_exec::block_on(async {
            let mut guard = rx_from_tx_steady.lock().await;
            guard.is_closed()
        });
        prop_assert!(tx_closed);
        let rx_open = core_exec::block_on(async {
            let mut guard = rx.lock().await;
            !guard.is_closed()
        });
        prop_assert!(rx_open);
    }
}
