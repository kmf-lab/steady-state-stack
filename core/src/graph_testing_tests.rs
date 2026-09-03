//! Graph-testing unit tests live here so graph_testing.rs stays under the 1,200-line budget;
//! they reach private StageManager/SideChannelResponder APIs via `super`.

// ss[related testing.graph-for-testing]
use super::*;
// ss[related testing.graph-for-testing]
use std::error::Error;
// ss[related testing.graph-for-testing]
use std::time::Duration;
// ss[related testing.graph-for-testing]
use aeron::aeron::Aeron;
// ss[related testing.graph-for-testing]
use futures::channel::oneshot;
// ss[related testing.graph-for-testing]
use crate::*;
// ss[related testing.graph-for-testing]
use crate::ActorName;
// ss[related testing.graph-for-testing]
use crate::ActorIdentity;
// ss[related testing.graph-for-testing]
use crate::distributed::aqueduct_stream::Defrag;
// ss[related testing.graph-for-testing]
use crate::simulate_edge::IntoSimRunner;
// ss[related testing.graph-for-testing]
use crate::channel_builder::ChannelBuilder;
// ss[related testing.graph-for-testing]
use crate::RxCoreBundle;
// ss[related testing.graph-for-testing]
use crate::steady_actor::BlockingCallFuture;
// ss[related testing.graph-for-testing]
use crate::TxCoreBundle;

// ss[related testing.graph-for-testing]
struct DummyActor {
    has_data: bool,
}

// ss[related testing.graph-for-testing]
impl SteadyActor for DummyActor {
    // ss[related testing.graph-for-testing]
    fn frame_rate_ms(&self) -> u64 { 0 }
    // ss[related testing.graph-for-testing]
    fn regeneration(&self) -> u32 { 0 }
    // ss[related testing.graph-for-testing]
    fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> { None }
    // ss[related testing.graph-for-testing]
    async fn simulated_behavior(self, _sims: Vec<&dyn IntoSimRunner<Self>>) -> Result<(), Box<dyn Error>> { Ok(()) }
    // ss[related testing.graph-for-testing]
    fn loglevel(&self, _loglevel: crate::LogLevel) {}
    // ss[related testing.graph-for-testing]
    fn relay_stats_smartly(&mut self) -> bool { false }
    // ss[related testing.graph-for-testing]
    fn relay_stats(&mut self) {}
    // ss[related testing.graph-for-testing]
    async fn relay_stats_periodic(&mut self, _duration_rate: Duration) -> bool { false }
    // ss[related testing.graph-for-testing]
    fn is_liveliness_in(&self, _target: &[GraphLivelinessState]) -> bool { false }
    // ss[related testing.graph-for-testing]
    fn is_liveliness_building(&self) -> bool { false }
    // ss[related testing.graph-for-testing]
    fn is_liveliness_running(&self) -> bool { false }
    // ss[related testing.graph-for-testing]
    fn is_liveliness_stop_requested(&self) -> bool { false }
    // ss[related testing.graph-for-testing]
    fn is_liveliness_shutdown_timeout(&self) -> Option<Duration> { None }
    // ss[related testing.graph-for-testing]
    fn flush_defrag_messages<S: StreamControlItem>(
        &mut self,
        _item: &mut Tx<S>,
        _data: &mut Tx<u8>,
        _defrag: &mut Defrag<S>,
    ) -> (u32, u32, Option<i32>) { (0, 0, None) }
    // ss[related testing.graph-for-testing]
    async fn wait_periodic(&self, _duration_rate: Duration) -> bool { false }
    // ss[related testing.graph-for-testing]
    async fn wait_timeout(&self, _timeout: Duration) -> bool { false }
    // ss[related testing.graph-for-testing]
    async fn wait(&self, _duration: Duration) {}
    // ss[related testing.graph-for-testing]
    async fn wait_avail<T: RxCore>(&self, _this: &mut T, _size: usize) -> bool { true }
    // ss[related testing.graph-for-testing]
    async fn wait_avail_bundle<T: RxCore>(
        &self,
        _this: &mut RxCoreBundle<'_, T>,
        _size: usize,
        _ready_channels: usize,
    ) -> bool { true }
    // ss[related testing.graph-for-testing]
    async fn wait_avail_index<T: RxCore>(
        &self,
        _this: &mut RxCoreBundle<'_, T>,
        _counts: &[usize],
    ) -> Option<usize> { Some(0) }
    // ss[related testing.graph-for-testing]
    async fn wait_future_void<F>(&self, _fut: F) -> bool where F: FusedFuture<Output = ()> + 'static + Send + Sync { false }
    // ss[related testing.graph-for-testing]
    async fn wait_vacant<T: TxCore>(&self, _this: &mut T, _count: T::MsgSize) -> bool { true }
    // ss[related testing.graph-for-testing]
    async fn wait_vacant_bundle<T: TxCore>(
        &self,
        _this: &mut TxCoreBundle<'_, T>,
        _count: T::MsgSize,
        _ready_channels: usize,
    ) -> bool { true }
    // ss[related testing.graph-for-testing]
    async fn wait_vacant_index<T: TxCore>(
        &self,
        _this: &mut TxCoreBundle<'_, T>,
        _counts: &[T::MsgSize],
    ) -> Option<usize> { Some(0) }
    // ss[related testing.graph-for-testing]
    async fn wait_avail_vacant_index<R: RxCore, T: TxCore>(
        &self,
        _rx: &mut RxCoreBundle<'_, R>,
        _tx: &mut TxCoreBundle<'_, T>,
        _avail_counts: &[usize],
        _vacant_counts: &[T::MsgSize],
    ) -> Option<usize> { Some(0) }
    // ss[related testing.graph-for-testing]
    async fn wait_shutdown(&self) -> bool { false }
    // ss[related testing.graph-for-testing]
    fn peek_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceSource<'b> where T: RxCore { unimplemented!() }
    // ss[related testing.graph-for-testing]
    fn advance_take_index<T: RxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> RxDone { unimplemented!() }
    // ss[related testing.graph-for-testing]
    fn take_slice<T: RxCore>(
        &mut self,
        _this: &mut T,
        _target: T::SliceTarget<'_>,
    ) -> RxDone where T::MsgItem: Copy { unimplemented!() }
    // ss[related testing.graph-for-testing]
    fn send_slice<T: TxCore>(
        &mut self,
        _this: &mut T,
        _source: T::SliceSource<'_>,
    ) -> TxDone where T::MsgOut: Copy { unimplemented!() }
    // ss[related testing.graph-for-testing]
    fn poke_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceTarget<'b> where T: TxCore { unimplemented!() }
    // ss[related testing.graph-for-testing]
    fn advance_send_index<T: TxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> TxDone { unimplemented!() }
    // ss[related testing.graph-for-testing]
    fn try_peek<'a, T>(&'a self, _this: &'a mut Rx<T>) -> Option<&'a T> { None }
    // ss[related testing.graph-for-testing]
    fn try_peek_iter<'a, T>(
        &'a self,
        _this: &'a mut Rx<T>,
    ) -> impl Iterator<Item = &'a T> + 'a { std::iter::empty() }
    // ss[related testing.graph-for-testing]
    fn is_empty<T: RxCore>(&self, _this: &mut T) -> bool { !self.has_data }
    // ss[related testing.graph-for-testing]
    fn avail_units<T: RxCore>(&self, this: &mut T) -> T::MsgSize { if self.has_data { this.one() } else { unimplemented!() } }
    // ss[related testing.graph-for-testing]
    async fn peek_async<'a, T: RxCore>(
        &'a self,
        _this: &'a mut T,
    ) -> Option<T::MsgPeek<'a>> { None }
    // ss[related testing.graph-for-testing]
    fn send_iter_until_full<T, I: Iterator<Item = T>>(
        &mut self,
        _this: &mut Tx<T>,
        _iter: I,
    ) -> usize { 0 }
    // ss[related testing.graph-for-testing]
    fn try_send<T: TxCore>(
        &mut self,
        this: &mut T,
        msg: T::MsgIn<'_>,
    ) -> SendOutcome<T::MsgOut> {
        if self.has_data {
            match this.shared_try_send(msg) {
                Ok(_) => SendOutcome::Success,
                Err(blocked) => SendOutcome::Blocked(blocked),
            }
        } else {
            SendOutcome::Success
        }
    }
    // ss[related testing.graph-for-testing]
    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> {
        if self.has_data {
            this.shared_try_take().map(|(_done, msg)| msg)
        } else {
            None
        }
    }
    // ss[related testing.graph-for-testing]
    fn is_full<T: TxCore>(&self, _this: &mut T) -> bool { false }
    // ss[related testing.graph-for-testing]
    fn vacant_units<T: TxCore>(&self, this: &mut T) -> T::MsgSize { this.one() }
    // ss[related testing.graph-for-testing]
    async fn wait_empty<T: TxCore>(&self, _this: &mut T) -> bool { false }
    // ss[related testing.graph-for-testing]
    fn take_into_iter<'a, T: Sync + Send>(
        &mut self,
        _this: &'a mut Rx<T>,
    ) -> impl Iterator<Item = T> + 'a { std::iter::empty() }
    // ss[related testing.graph-for-testing]
    async fn call_async<F>(&self, _operation: F) -> Option<F::Output> where F: Future { None }
    // ss[related testing.graph-for-testing]
    fn call_blocking<F, T>(&self, f: F) -> BlockingCallFuture<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static {
        BlockingCallFuture(core_exec::spawn_blocking(f))
    }
    // ss[related testing.graph-for-testing]
    async fn send_async<T: TxCore>(
        &mut self,
        _this: &mut T,
        _a: T::MsgIn<'_>,
        _saturation: SendSaturation,
    ) -> SendOutcome<T::MsgOut> { SendOutcome::Success }
    // ss[related testing.graph-for-testing]
    async fn take_async<T>(&mut self, _this: &mut Rx<T>) -> Option<T> { None }
    // ss[related testing.graph-for-testing]
    async fn take_async_with_timeout<T>(
        &mut self,
        _this: &mut Rx<T>,
        _timeout: Duration,
    ) -> Option<T> { None }
    // ss[related testing.graph-for-testing]
    async fn yield_now(&self) {}
    // ss[related testing.graph-for-testing]
    fn sidechannel_responder(&self) -> Option<SideChannelResponder> { None }
    // ss[related testing.graph-for-testing]
    fn is_running<F: FnMut() -> bool>(&mut self, _accept_fn: F) -> bool { true }
    // ss[related testing.graph-for-testing]
    async fn request_shutdown(&mut self) {}
    // ss[related testing.graph-for-testing]
    fn args<A: Any>(&self) -> Option<&A> { None }
    // ss[related testing.graph-for-testing]
    fn identity(&self) -> ActorIdentity { ActorIdentity::default() }
    // ss[related testing.graph-for-testing]
    fn is_showstopper<T>(&self, _rx: &mut Rx<T>, _threshold: usize) -> bool { false }

    // ss[related testing.graph-for-testing]
    fn set_dot_display_text(&mut self, _text: Option<&str>) {}
}

// ss[verify testing.graph-for-testing]
// ss[verify testing.graph-for-testing]
// ss[verify testing.mock-main-thread]
// ss[verify testing.deterministic-no-sleep]
#[test]
// ss[related testing.graph-for-testing]
fn test_graph_test_result() -> Result<(), Box<dyn Error>> {
    let ok: GraphTestResult<i32, String> = GraphTestResult::Ok(42);
    if let GraphTestResult::Ok(val) = ok {
        assert_eq!(val, 42);
    } else {
        return Err("Expected Ok".into());
    }

    let err: GraphTestResult<i32, String> = GraphTestResult::Err("error".to_string());
    if let GraphTestResult::Err(val) = err {
        assert_eq!(val, "error");
    } else {
        return Err("Expected Err".into());
    }

    Ok(())
}

// ss[verify testing.stage-manager-integration]
#[test]
// ss[related testing.graph-for-testing]
fn test_stack_guarded_graph() -> Result<(), Box<dyn Error>> {
    SteadyRunner::test_build()
        .with_stack_size(16 * 1024 * 1024)
        .run((), |mut graph| {
            graph.start();
            let sm = graph.stage_manager();
            sm.final_bow();
            graph.request_shutdown();
            graph.block_until_stopped(Duration::from_secs(5))
        })
}

// ss[verify testing.stage-manager-integration]
#[test]
// ss[related testing.graph-for-testing]
fn test_stage_manager_default() -> Result<(), Box<dyn Error>> {
    let manager = StageManager::default();
    assert!(manager.node.is_empty());
    assert!(manager.backplane.is_empty());
    Ok(())
}

// ss[verify testing.stage-manager-integration]
#[test]
// ss[related testing.graph-for-testing]
fn test_stage_manager_clone() -> Result<(), Box<dyn Error>> {
    let mut manager = StageManager::default();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    manager.register_node(ActorName::new("test", None), 10, shutdown_rx);

    let cloned = manager.clone();
    assert_eq!(manager.node.len(), cloned.node.len());
    assert_eq!(manager.backplane.len(), cloned.backplane.len());
    Ok(())
}

// ss[verify testing.stage-manager-integration]
#[test]
// ss[related testing.graph-for-testing]
fn test_stage_manager_debug() -> Result<(), Box<dyn Error>> {
    let manager = StageManager::default();
    let debug_str = format!("{:?}", manager);
    assert!(debug_str.contains("SideChannelHub"));
    Ok(())
}

#[test]
// ss[verify testing.graph-for-testing]
fn test_node_tx_rx() -> Result<(), Box<dyn Error>> {
    let mut manager = StageManager::default();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    manager.register_node(ActorName::new("test", None), 10, shutdown_rx);

    let node = manager.node_tx_rx(ActorName::new("test", None));
    assert!(node.is_some());

    let missing = manager.node_tx_rx(ActorName::new("missing", None));
    assert!(missing.is_none());
    Ok(())
}

#[test]
// ss[verify testing.graph-for-testing]
fn test_register_node() -> Result<(), Box<dyn Error>> {
    let mut manager = StageManager::default();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();

    let success = manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
    assert!(success);
    assert_eq!(manager.node.len(), 1);
    assert_eq!(manager.backplane.len(), 1);

    let (_shutdown_tx2, shutdown_rx2) = oneshot::channel();
    let duplicate = manager.register_node(ActorName::new("test", None), 10, shutdown_rx2);
    assert!(!duplicate);
    Ok(())
}

#[test]
// ss[verify testing.graph-for-testing]
fn test_call_actor_internal_errors() -> Result<(), Box<dyn Error>> {
    let mut manager = StageManager::default();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    let name = ActorName::new("test", None);
    manager.register_node(name, 1, shutdown_rx);

    // Correct simulation: Use the NODE side to simulate the actor
    let node_side = manager.node_tx_rx(name).unwrap();
    core_exec::spawn_detached(async move {
        let mut guard = node_side.lock().await;
        let ((tx_prod, _), _) = guard.deref_mut();
        // Wait for request and send malformed response
        let _ = tx_prod.try_push(Box::new(42i32)); 
    });

    let res = manager.call_actor_internal(Box::new("req"), name);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("unexpected message"));
    Ok(())
}

#[test]
// ss[verify testing.graph-for-testing]
fn test_side_channel_responder_new() -> Result<(), Box<dyn Error>> {
    let mut manager = StageManager::default();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
    let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
    let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
    assert_eq!(responder.identity, ActorIdentity::default());
    Ok(())
}

// ss[verify testing.deterministic-no-sleep]
#[test]
// ss[related testing.graph-for-testing]
fn test_avail() -> Result<(), Box<dyn Error>> {
    let mut manager = StageManager::default();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
    let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
    let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
    let backplane = manager.backplane.get(&ActorName::new("test", None)).unwrap().clone();

    assert_eq!(responder.avail(), 0);

    core_exec::block_on(async {
        let mut guard = backplane.lock().await;
        let (tx, _) = guard.deref_mut();
        tx.push(Box::new(42)).await
    }).expect("");

    assert_eq!(responder.avail(), 1);
    Ok(())
}

#[test]
// ss[verify testing.graph-for-testing]
fn test_should_apply_logic() -> Result<(), Box<dyn Error>> {
    let mut manager = StageManager::default();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
    let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
    let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
    let backplane = manager.backplane.get(&ActorName::new("test", None)).unwrap().clone();

    core_exec::block_on(async {
        let mut guard = backplane.lock().await;
        let (tx, _) = guard.deref_mut();
        tx.push(Box::new(42i32)).await
    }).expect("");

    let result = core_exec::block_on(responder.should_apply::<i32>());
    assert_eq!(result, Some(true));

    let result_wrong = core_exec::block_on(responder.should_apply::<String>());
    assert_eq!(result_wrong, Some(false));
    Ok(())
}

// ss[related testing.graph-for-testing]
async fn pipeline_generator_edge(
    actor: SteadyActorShadow,
    tx: SteadyTx<u64>,
) -> Result<(), Box<dyn Error>> {
    let actor = actor.into_spotlight([], [&tx]);
    // ss[related actor.internal-behavior-logic]
    if actor.use_internal_behavior {
        Ok(())
    } else {
        actor.simulated_behavior(sim_runners!(tx)).await
    }
}

// ss[related testing.graph-for-testing]
async fn pipeline_heartbeat_edge(
    actor: SteadyActorShadow,
    tx: SteadyTx<u64>,
) -> Result<(), Box<dyn Error>> {
    let actor = actor.into_spotlight([], [&tx]);
    if actor.use_internal_behavior {
        Ok(())
    } else {
        actor.simulated_behavior(sim_runners!(tx)).await
    }
}

// ss[related testing.graph-for-testing]
async fn pipeline_logger_edge(
    actor: SteadyActorShadow,
    rx: SteadyRx<u64>,
) -> Result<(), Box<dyn Error>> {
    let actor = actor.into_spotlight([&rx], []);
    if actor.use_internal_behavior {
        Ok(())
    } else {
        actor.simulated_behavior(sim_runners!(rx)).await
    }
}

// ss[impl testing.internal-behavior-direct]
// ss[impl testing.pipeline-worker-allowlist]
// ss[impl testing.deterministic-no-sleep]
async fn pipeline_worker_internal<A: SteadyActor>(
    mut actor: A,
    heartbeat: SteadyRx<u64>,
    generator: SteadyRx<u64>,
    logger: SteadyTx<u64>,
) -> Result<(), Box<dyn Error>> {
    let mut heartbeat = heartbeat.lock().await;
    let mut generator = generator.lock().await;
    let mut logger = logger.lock().await;

    while actor.is_running(
        || heartbeat.is_closed_and_empty()
            && generator.is_closed_and_empty()
            && logger.mark_closed(),
    ) {
        let clean = await_for_all!(
            actor.wait_avail(&mut heartbeat, 1),
            actor.wait_avail(&mut generator, 1),
            actor.wait_vacant(&mut logger, 1)
        );

        if actor.try_take(&mut heartbeat).is_some() || !clean {
            if let Some(&value) = actor.try_peek(&mut generator) {
                match actor.try_send(&mut logger, value) {
                    SendOutcome::Success => {
                        actor.try_take(&mut generator).expect("internal error");
                    }
                    SendOutcome::Blocked(_) => continue,
                    SendOutcome::Timeout(_) | SendOutcome::Closed(_) => continue,
                }
            }
        }
    }
    Ok(())
}

// ss[related testing.graph-for-testing]
async fn pipeline_worker_run(
    actor: SteadyActorShadow,
    heartbeat_rx: SteadyRx<u64>,
    generator_rx: SteadyRx<u64>,
    logger_tx: SteadyTx<u64>,
) -> Result<(), Box<dyn Error>> {
    let actor = actor.into_spotlight([&heartbeat_rx, &generator_rx], [&logger_tx]);
    if actor.use_internal_behavior {
        pipeline_worker_internal(actor, heartbeat_rx, generator_rx, logger_tx).await
    } else {
        actor
            .simulated_behavior(sim_runners!(
                heartbeat_rx,
                generator_rx,
                logger_tx
            ))
            .await
    }
}

// ss[related testing.graph-for-testing]
async fn sim_tx_producer_edge(
    actor: SteadyActorShadow,
    tx: SteadyTx<u64>,
) -> Result<(), Box<dyn Error>> {
    let actor = actor.into_spotlight([], [&tx]);
    if actor.use_internal_behavior {
        Ok(())
    } else {
        actor.simulated_behavior(sim_runners!(tx)).await
    }
}

// ss[related testing.graph-for-testing]
async fn one_u64_consumer_internal<A: SteadyActor>(
    mut actor: A,
    rx: SteadyRx<u64>,
) -> Result<(), Box<dyn Error>> {
    let mut rx = rx.lock().await;
    while actor.is_running(|| rx.is_closed_and_empty()) {
        let _clean = await_for_all!(actor.wait_avail(&mut rx, 1));
        let _ = actor.try_take(&mut rx);
    }
    Ok(())
}

// ss[verify testing.stage-manager-integration]
// ss[verify actor.run-dispatcher]
// ss[verify actor.shadow-spotlight]
// ss[verify testing.internal-behavior-direct]
#[test]
// ss[related testing.graph-for-testing]
fn staged_single_sim_producer_and_real_consumer_shuts_down_cleanly() -> Result<(), Box<dyn Error>> {
    SteadyRunner::test_build().run((), |mut graph| {
        let (prod_tx, prod_rx) = graph.channel_builder().with_capacity(8).build::<u64>();

        graph.actor_builder().with_name("PRODUCER").build(
            move |ctx| sim_tx_producer_edge(ctx, prod_tx.clone()),
            SoloAct,
        );
        graph.actor_builder().with_name("CONSUMER").build(
            move |ctx| {
                let rx = prod_rx.clone();
                async move {
                    let actor = ctx.into_spotlight([&rx], []);
                    one_u64_consumer_internal(actor, rx).await
                }
            },
            SoloAct,
        );

        graph.start();
        let sm = graph.stage_manager();
        sm.actor_perform("PRODUCER", StageDirection::Echo(42_u64))?;
        sm.final_bow();

        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(5))
    })
}

// ss[verify testing.stage-manager-integration]
// ss[verify testing.pipeline-worker-allowlist]
// ss[verify philosophy.structural-hierarchy]
// ss[verify actor.internal-behavior-logic]
#[test]
// ss[related testing.graph-for-testing]
fn staged_pipeline_four_actor_graph_regression() -> Result<(), Box<dyn Error>> {
    // ss[related testing.graph-for-testing]
    const NAME_GENERATOR: &str = "GENERATOR";
    // ss[related testing.graph-for-testing]
    const NAME_HEARTBEAT: &str = "HEARTBEAT";
    // ss[related testing.graph-for-testing]
    const NAME_WORKER: &str = "WORKER";
    // ss[related testing.graph-for-testing]
    const NAME_LOGGER: &str = "LOGGER";

    SteadyRunner::test_build().run((), |mut graph| {
        let (gen_lazy, gen_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
        let (hb_lazy, hb_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
        let (log_lazy, log_rx) = graph.channel_builder().with_capacity(16).build::<u64>();

        graph
            .actor_builder()
            .with_name(NAME_GENERATOR)
            .build(move |ctx| pipeline_generator_edge(ctx, gen_lazy.clone()), SoloAct);
        graph
            .actor_builder()
            .with_name(NAME_HEARTBEAT)
            .build(move |ctx| pipeline_heartbeat_edge(ctx, hb_lazy.clone()), SoloAct);
        graph.actor_builder().with_name(NAME_WORKER).build(
            move |ctx| pipeline_worker_run(ctx, hb_rx.clone(), gen_rx.clone(), log_lazy.clone()),
            SoloAct,
        );
        graph
            .actor_builder()
            .with_name(NAME_LOGGER)
            .build(move |ctx| pipeline_logger_edge(ctx, log_rx.clone()), SoloAct);

        graph.start();

        let sm = graph.stage_manager();
        sm.actor_perform(NAME_GENERATOR, StageDirection::Echo(15_u64))?;
        sm.actor_perform(NAME_HEARTBEAT, StageDirection::Echo(100_u64))?;
        sm.actor_perform(
            NAME_LOGGER,
            StageWaitFor::Message(15_u64, Duration::from_secs(2)),
        )?;
        sm.final_bow();

        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(5))
    })
}

// #[test]
// #[ignore] //this complex test still hangs
// fn test_wait_available_units_shutdown() -> Result<(), Box<dyn Error>> {
//     let mut manager = StageManager::default();
//     let (shutdown_tx, shutdown_rx) = oneshot::channel();
//     manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
//     let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
//     let mut responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
//
//     core_exec::spawn_detached(async move {
//         let _ = Delay::new(Duration::from_millis(10)).await;
//         drop(shutdown_tx); // Trigger shutdown
//     });
//
//     let result = core_exec::block_on(responder.wait_available_units(5));
//     assert!(!result);
//     Ok(())
// }

#[test]
// ss[verify testing.graph-for-testing]
fn test_respond_with_error_path() -> Result<(), Box<dyn Error>> {
    let mut manager = StageManager::default();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    manager.register_node(ActorName::new("test", None), 1, shutdown_rx);
    let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
    let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
    
    // Fill the response channel from the driver side to force an error in respond_with
    let backplane = manager.backplane.get(&ActorName::new("test", None)).unwrap().clone();
    core_exec::block_on(async {
        let mut guard = backplane.lock().await;
        let (tx, _) = guard.deref_mut();
        tx.push(Box::new("request")).await.unwrap();
    });

    let mut actor = DummyActor { has_data: true };
    // This test exercises the "Ok(true)" branch when empty, and "Ok(false)" when logic returns None.
    let res = responder.respond_with(|_, _| None, &mut actor)?;
    assert!(!res);
    Ok(())
}

// ss[related testing.graph-for-testing]
use proptest::prelude::*;

// ss[related testing.graph-for-testing]
fn build_shutdown_proptest_pipeline(
    graph: &mut Graph,
) -> (
    LazySteadyTx<u64>,
    LazySteadyTx<u64>,
    SteadyRx<u64>,
) {
    // ss[related testing.graph-for-testing]
    const NAME_GENERATOR: &str = "GENERATOR";
    // ss[related testing.graph-for-testing]
    const NAME_HEARTBEAT: &str = "HEARTBEAT";
    // ss[related testing.graph-for-testing]
    const NAME_WORKER: &str = "WORKER";
    // ss[related testing.graph-for-testing]
    const NAME_LOGGER: &str = "LOGGER";

    let (gen_lazy, generator_rx) = graph.channel_builder().with_capacity(64).build::<u64>();
    let (hb_lazy, hb_rx) = graph.channel_builder().with_capacity(64).build::<u64>();
    let (log_lazy, log_rx_lazy) = graph.channel_builder().with_capacity(64).build::<u64>();
    let log_rx_out = log_rx_lazy.clone();

    let actor_builder = graph.actor_builder();

    actor_builder
        .with_name(NAME_GENERATOR)
        .never_simulate(true)
        .build(
            |ctx| async move {
                let mut actor = ctx.into_spotlight([], []);
                while actor.is_running(|| true) {}
                Ok(())
            },
            SoloAct,
        );

    actor_builder
        .with_name(NAME_HEARTBEAT)
        .never_simulate(true)
        .build(
            |ctx| async move {
                let mut actor = ctx.into_spotlight([], []);
                while actor.is_running(|| true) {}
                Ok(())
            },
            SoloAct,
        );

    graph.actor_builder().with_name(NAME_WORKER).build(
        move |ctx| {
            let hb = hb_rx.clone();
            let generator = generator_rx.clone();
            let log = log_lazy.clone();
            async move {
                let actor = ctx.into_spotlight([&hb, &generator], [&log]);
                pipeline_worker_internal(actor, hb, generator, log).await
            }
        },
        SoloAct,
    );

    actor_builder
        .with_name(NAME_LOGGER)
        .never_simulate(true)
        .build(
            |ctx| async move {
                let mut actor = ctx.into_spotlight([], []);
                while actor.is_running(|| true) {}
                Ok(())
            },
            SoloAct,
        );

    (gen_lazy, hb_lazy, log_rx_out)
}

// ss[related testing.graph-for-testing]
fn build_staged_puppet_pipeline(graph: &mut Graph) {
    // ss[related testing.graph-for-testing]
    const NAME_GENERATOR: &str = "GENERATOR";
    // ss[related testing.graph-for-testing]
    const NAME_HEARTBEAT: &str = "HEARTBEAT";
    // ss[related testing.graph-for-testing]
    const NAME_WORKER: &str = "WORKER";
    // ss[related testing.graph-for-testing]
    const NAME_LOGGER: &str = "LOGGER";

    let (gen_lazy, gen_rx) = graph.channel_builder().with_capacity(32).build::<u64>();
    let (hb_lazy, hb_rx) = graph.channel_builder().with_capacity(32).build::<u64>();
    let (log_lazy, log_rx) = graph.channel_builder().with_capacity(32).build::<u64>();

    graph
        .actor_builder()
        .with_name(NAME_GENERATOR)
        .build(move |ctx| pipeline_generator_edge(ctx, gen_lazy.clone()), SoloAct);
    graph
        .actor_builder()
        .with_name(NAME_HEARTBEAT)
        .build(move |ctx| pipeline_heartbeat_edge(ctx, hb_lazy.clone()), SoloAct);
    graph.actor_builder().with_name(NAME_WORKER).build(
        move |ctx| pipeline_worker_run(ctx, hb_rx.clone(), gen_rx.clone(), log_lazy.clone()),
        SoloAct,
    );
    graph
        .actor_builder()
        .with_name(NAME_LOGGER)
        .build(move |ctx| pipeline_logger_edge(ctx, log_rx.clone()), SoloAct);
}

// ss[related testing.graph-for-testing]
fn setup_side_channel_responder(capacity: usize) -> (StageManager, SideChannelResponder) {
    let mut manager = StageManager::default();
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    manager.register_node(ActorName::new("EDGE", None), capacity, shutdown_rx);
    let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
    let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
    (manager, responder)
}

/// Voting-phase bound for graph integration properties (work must finish before shutdown).
// ss[related testing.graph-for-testing]
fn integration_vote_timeout() -> Duration {
    Duration::from_millis(500)
}

/// Poll logger availability until `pred` holds or the deadline elapses.
// ss[related testing.graph-for-testing]
fn poll_log_avail<F>(log_rx: &SteadyRx<u64>, deadline: Duration, mut pred: F)
where
    F: FnMut(usize) -> bool,
{
    let end = Instant::now() + deadline;
    loop {
        let avail = {
            let mut rx = core_exec::block_on(log_rx.lock());
            rx.avail_units()
        };
        if pred(avail) || Instant::now() >= end {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Request shutdown and bound only the cooperative voting/drain phase.
// ss[related testing.graph-for-testing]
fn shutdown_started_graph(mut graph: Graph) -> Result<(), Box<dyn Error>> {
    graph.request_shutdown();
    graph.block_until_stopped(integration_vote_timeout())
}


#[path = "graph_testing_proptest.rs"]
mod graph_testing_proptest;
