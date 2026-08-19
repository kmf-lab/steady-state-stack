// ss[related testing.sim-producer-close]
use std::error::Error;
// ss[related philosophy.structural-hierarchy]
use std::fmt::Debug;
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related testing.sim-producer-close]
use std::time::Duration;
// ss[related philosophy.structural-hierarchy]
use futures_util::lock::Mutex;
// ss[related philosophy.structural-hierarchy]
use log::*;
// ss[related testing.sim-producer-close]
use crate::distributed::aqueduct_stream::{Defrag, StreamControlItem, StreamRx, StreamTx};
// ss[related philosophy.structural-hierarchy]
use crate::graph_testing::SideChannelResponder;
// ss[related philosophy.structural-hierarchy]
use crate::steady_actor::{BlockingCallFuture, SendOutcome};
// ss[related testing.sim-producer-close]
use crate::steady_actor_core::SteadyActorCore;
// ss[related testing.sim-producer-close]
use crate::core_rx::RxCore;
// ss[related philosophy.structural-hierarchy]
use crate::core_tx::TxCore;
// ss[related philosophy.structural-hierarchy]
use crate::steady_rx::{Rx, RxDone};
// ss[related testing.sim-producer-close]
use crate::steady_tx::{Tx, TxDone};
// ss[related philosophy.structural-hierarchy]
use crate::yield_now::yield_now;
// ss[related philosophy.structural-hierarchy]
use crate::core_exec;
// ss[related testing.sim-producer-close]
use crate::{ActorIdentity, GraphLivelinessState, RxCoreBundle, SendSaturation, SteadyActor, SteadyRx, SteadyTx, TxCoreBundle};
// ss[related philosophy.structural-hierarchy]
use aeron::aeron::Aeron;
// ss[related philosophy.structural-hierarchy]
use futures_util::future::FusedFuture;
// ss[related testing.sim-producer-close]
use std::any::Any;
// ss[related philosophy.structural-hierarchy]
use std::future::Future;

/// The `SimRunner` trait defines the interface for actors that can be simulated in edge case tests.
// ss[related testing.sim-producer-close]
pub trait SimRunner<C: SteadyActor + ?Sized> {
    /// Called each simulation iteration. Return `SimStepResult::DidWork` if work was done,
    /// `SimStepResult::NoWork` if no work was done.
    // ss[related testing.sim-producer-close]
    fn step(&mut self) -> Result<SimStepResult, Box<dyn Error>>;

    /// Stage-manager integration: handle [`crate::graph_testing::StageDirection`] / [`crate::graph_testing::StageWaitFor`]
    /// on the side channel for this runner's channel slot.
    // ss[related testing.sim-producer-close]
    fn stage_step(
        &mut self,
        actor: &mut C,
        responder: &SideChannelResponder,
    ) -> Result<SimStepResult, Box<dyn Error>> {
        let _ = (actor, responder);
        Ok(SimStepResult::NoWork)
    }

    // ss[impl testing.sim-producer-close]
    /// After [`simulated_behavior`]'s loop exits successfully, mark this runner's simulated **outputs**
    /// closed so downstream actors can satisfy shutdown vetoes (e.g. `rx.is_closed_and_empty()`).
    ///
    /// Default: no-op. Implemented for transmit-side runners (`SimTx`, `SimTxBundle`, `SimStreamTx`).
    // ss[related testing.sim-producer-close]
    fn close_outputs_on_simulated_stop(&mut self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}

/// Result of a single simulation step.
#[derive(Debug, PartialEq, Eq)]
// ss[related testing.sim-producer-close]
pub enum SimStepResult {
    /// Work was performed during this step.
    DidWork,
    /// No work was performed (e.g., channel was empty or full).
    NoWork,
}

/// Trait for converting channels (or bundles) into simulation runners.
// ss[related testing.sim-producer-close]
pub trait IntoSimRunner<C: SteadyActor + ?Sized> {
    /// Converts this channel/bundle into a `SimRunner` that can be driven by `simulated_behavior`.
    // ss[related philosophy.structural-hierarchy]
    fn into_sim_runner(&self) -> Box<dyn SimRunner<C>>;
}

// ss[related testing.sim-producer-close]
pub(crate) async fn simulated_behavior<C: SteadyActor>(
    actor: &mut C,
    sims: Vec<&dyn IntoSimRunner<C>>,
) -> Result<(), Box<dyn Error>> {
    let mut sim_runners: Vec<Box<dyn SimRunner<C>>> =
        sims.into_iter().map(|s| s.into_sim_runner()).collect();

    while actor.is_running(|| true) {
        let mut did_work = false;
        if let Some(responder) = actor.sidechannel_responder() {
            // Integration / StageManager mode: only `stage_step` may touch channels. Plain `step()`
            // auto-sends or discards messages and races `simulate_direction` / `simulate_wait_for`.
            for runner in sim_runners.iter_mut() {
                match runner.stage_step(actor, &responder) {
                    Ok(SimStepResult::DidWork) => {
                        did_work = true;
                    }
                    Ok(SimStepResult::NoWork) => {}
                    Err(e) => {
                        warn!("Simulation stage_step error: {:?}", e);
                        return Err(e);
                    }
                }
            }
        } else {
            for runner in sim_runners.iter_mut() {
                match runner.step() {
                    Ok(SimStepResult::DidWork) => {
                        did_work = true;
                    }
                    Ok(SimStepResult::NoWork) => {}
                    Err(e) => {
                        warn!("Simulation step error: {:?}", e);
                        return Err(e);
                    }
                }
            }
        }
        if !did_work {
            actor.yield_now().await;
        }
    }
    for runner in sim_runners.iter_mut() {
        runner.close_outputs_on_simulated_stop()?;
    }
    Ok(())
}

/// Implementation for `SteadyRx` (single receiver) as a simulation runner.
// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: 'static + Send + Debug + Clone + Eq> IntoSimRunner<C> for Arc<Mutex<Rx<T>>> {
    // ss[related philosophy.structural-hierarchy]
    fn into_sim_runner(&self) -> Box<dyn SimRunner<C>> {
        Box::new(SimRx::new(self.clone()))
    }
}

// ss[related testing.sim-producer-close]
struct SimRx<T> {
    rx: Arc<Mutex<Rx<T>>>,
}

// ss[related testing.sim-producer-close]
impl<T: 'static + Send + Debug + Clone> SimRx<T> {
    // ss[related philosophy.structural-hierarchy]
    fn new(rx: Arc<Mutex<Rx<T>>>) -> Self {
        SimRx { rx }
    }
}

// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: 'static + Send + Debug + Clone + Eq> SimRunner<C> for SimRx<T> {
    // ss[related philosophy.structural-hierarchy]
    fn step(&mut self) -> Result<SimStepResult, Box<dyn Error>> {
        if let Some(mut guard) = self.rx.try_lock() {
            if guard.shared_avail_units() > 0 {
                guard.shared_try_take();
                Ok(SimStepResult::DidWork)
            } else {
                Ok(SimStepResult::NoWork)
            }
        } else {
            Ok(SimStepResult::NoWork)
        }
    }

    // ss[related testing.sim-producer-close]
    fn stage_step(
        &mut self,
        actor: &mut C,
        responder: &SideChannelResponder,
    ) -> Result<SimStepResult, Box<dyn Error>> {
        if let Some(mut guard) = self.rx.try_lock() {
            responder.simulate_wait_for(&mut *guard, actor, 0)
        } else {
            Ok(SimStepResult::NoWork)
        }
    }
}

/// Implementation for `SteadyTx` (single transmitter) as a simulation runner.
// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: 'static + Send + Sync + Debug + Clone + Default> IntoSimRunner<C> for Arc<Mutex<Tx<T>>> {
    // ss[related philosophy.structural-hierarchy]
    fn into_sim_runner(&self) -> Box<dyn SimRunner<C>> {
        Box::new(SimTx::new(self.clone()))
    }
}

// ss[related testing.sim-producer-close]
struct SimTx<T> {
    tx: Arc<Mutex<Tx<T>>>,
    msg: Option<T>,
}

// ss[related testing.sim-producer-close]
impl<T: 'static + Send + Sync + Debug + Clone + Default> SimTx<T> {
    // ss[related philosophy.structural-hierarchy]
    fn new(tx: Arc<Mutex<Tx<T>>>) -> Self {
        SimTx { tx, msg: None }
    }
}

// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: 'static + Send + Sync + Debug + Clone + Default> SimRunner<C> for SimTx<T> {
    // ss[related philosophy.structural-hierarchy]
    fn step(&mut self) -> Result<SimStepResult, Box<dyn Error>> {
        if let Some(mut guard) = self.tx.try_lock() {
            if !guard.shared_is_full() {
                let msg = self.msg.clone().unwrap_or_default();
                let _ = guard.shared_try_send(msg.clone());
                self.msg = Some(msg);
                Ok(SimStepResult::DidWork)
            } else {
                Ok(SimStepResult::NoWork)
            }
        } else {
            Ok(SimStepResult::NoWork)
        }
    }

    // ss[related testing.sim-producer-close]
    fn stage_step(
        &mut self,
        actor: &mut C,
        responder: &SideChannelResponder,
    ) -> Result<SimStepResult, Box<dyn Error>> {
        if let Some(mut guard) = self.tx.try_lock() {
            responder.simulate_direction(&mut *guard, actor, 0)
        } else {
            Ok(SimStepResult::NoWork)
        }
    }

    // ss[related testing.sim-producer-close]
    fn close_outputs_on_simulated_stop(&mut self) -> Result<(), Box<dyn Error>> {
        core_exec::block_on(async {
            let mut g = self.tx.lock().await;
            g.mark_closed();
        });
        Ok(())
    }
}

/// Implementation for `SteadyStreamRx` (receiver side of a stream) as a simulation runner.
// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: StreamControlItem> IntoSimRunner<C> for Arc<Mutex<StreamRx<T>>> {
    // ss[related philosophy.structural-hierarchy]
    fn into_sim_runner(&self) -> Box<dyn SimRunner<C>> {
        Box::new(SimStreamRx::new(self.clone()))
    }
}

// ss[related testing.sim-producer-close]
struct SimStreamRx<T: StreamControlItem> {
    rx: Arc<Mutex<StreamRx<T>>>,
}

// ss[related testing.sim-producer-close]
impl<T: StreamControlItem> SimStreamRx<T> {
    // ss[related philosophy.structural-hierarchy]
    fn new(rx: Arc<Mutex<StreamRx<T>>>) -> Self {
        SimStreamRx { rx }
    }
}

// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: StreamControlItem> SimRunner<C> for SimStreamRx<T> {
    // ss[related philosophy.structural-hierarchy]
    fn step(&mut self) -> Result<SimStepResult, Box<dyn Error>> {
        if let Some(mut guard) = self.rx.try_lock() {
            if guard.control_channel.shared_avail_units() > 0 {
                guard.control_channel.shared_try_take();
                Ok(SimStepResult::DidWork)
            } else {
                Ok(SimStepResult::NoWork)
            }
        } else {
            Ok(SimStepResult::NoWork)
        }
    }
}

/// Implementation for `SteadyStreamTx` (transmitter side of a stream) as a simulation runner.
// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: StreamControlItem> IntoSimRunner<C> for Arc<Mutex<StreamTx<T>>> {
    // ss[related philosophy.structural-hierarchy]
    fn into_sim_runner(&self) -> Box<dyn SimRunner<C>> {
        Box::new(SimStreamTx::new(self.clone()))
    }
}

// ss[related testing.sim-producer-close]
struct SimStreamTx<T: StreamControlItem> {
    tx: Arc<Mutex<StreamTx<T>>>,
}

// ss[related testing.sim-producer-close]
impl<T: StreamControlItem> SimStreamTx<T> {
    // ss[related philosophy.structural-hierarchy]
    fn new(tx: Arc<Mutex<StreamTx<T>>>) -> Self {
        SimStreamTx { tx }
    }
}

// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: StreamControlItem> SimRunner<C> for SimStreamTx<T> {
    // ss[related philosophy.structural-hierarchy]
    fn step(&mut self) -> Result<SimStepResult, Box<dyn Error>> {
        if let Some(mut guard) = self.tx.try_lock() {
            let ctrl_vacant = guard.control_channel.shared_vacant_units();
            if ctrl_vacant > 0 {
                let dummy = T::testing_new(8);
                let payload = vec![0u8; 8];
                let _ = guard.control_channel.shared_try_send(dummy);
                guard.payload_channel.shared_send_slice(&payload);
                Ok(SimStepResult::DidWork)
            } else {
                Ok(SimStepResult::NoWork)
            }
        } else {
            Ok(SimStepResult::NoWork)
        }
    }

    // ss[related testing.sim-producer-close]
    fn close_outputs_on_simulated_stop(&mut self) -> Result<(), Box<dyn Error>> {
        core_exec::block_on(async {
            let mut g = self.tx.lock().await;
            g.mark_closed();
        });
        Ok(())
    }
}

/// Implementation for `SteadyRxBundle` (bundle of receivers) as a simulation runner.
// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: 'static + Send + Debug + Clone + Eq, const N: usize> IntoSimRunner<C>
    for Arc<[SteadyRx<T>; N]>
{
    // ss[related testing.sim-producer-close]
    fn into_sim_runner(&self) -> Box<dyn SimRunner<C>> {
        Box::new(SimRxBundle::new(self.clone()))
    }
}

// ss[related testing.sim-producer-close]
struct SimRxBundle<T, const N: usize> {
    rx_bundle: Arc<[SteadyRx<T>; N]>,
    index: usize,
}

// ss[related testing.sim-producer-close]
impl<T: 'static + Send + Debug + Clone + Eq, const N: usize> SimRxBundle<T, N> {
    // ss[related philosophy.structural-hierarchy]
    fn new(rx_bundle: Arc<[SteadyRx<T>; N]>) -> Self {
        SimRxBundle { rx_bundle, index: 0 }
    }
}

// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: 'static + Send + Debug + Clone + Eq, const N: usize> SimRunner<C>
    for SimRxBundle<T, N>
{
    // ss[related testing.sim-producer-close]
    fn step(&mut self) -> Result<SimStepResult, Box<dyn Error>> {
        let i = self.index % N;
        let rx = &self.rx_bundle[i];
        self.index += 1;
        if let Some(mut guard) = rx.try_lock() {
            if guard.shared_avail_units() > 0 {
                guard.shared_try_take();
                Ok(SimStepResult::DidWork)
            } else {
                Ok(SimStepResult::NoWork)
            }
        } else {
            Ok(SimStepResult::NoWork)
        }
    }

    // ss[related testing.sim-producer-close]
    fn stage_step(
        &mut self,
        actor: &mut C,
        responder: &SideChannelResponder,
    ) -> Result<SimStepResult, Box<dyn Error>> {
        for lane in 0..N {
            if let Some(mut guard) = self.rx_bundle[lane].try_lock() {
                let r = responder.simulate_wait_for(&mut *guard, actor, lane)?;
                if r == SimStepResult::DidWork {
                    return Ok(SimStepResult::DidWork);
                }
            }
        }
        Ok(SimStepResult::NoWork)
    }
}

/// Implementation for `SteadyTxBundle` (bundle of transmitters) as a simulation runner.
// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: 'static + Send + Sync + Debug + Clone + Default, const N: usize> IntoSimRunner<C>
    for Arc<[SteadyTx<T>; N]>
{
    // ss[related testing.sim-producer-close]
    fn into_sim_runner(&self) -> Box<dyn SimRunner<C>> {
        Box::new(SimTxBundle::new(self.clone()))
    }
}

// ss[related testing.sim-producer-close]
struct SimTxBundle<T, const N: usize> {
    tx_bundle: Arc<[SteadyTx<T>; N]>,
    index: usize,
}

// ss[related testing.sim-producer-close]
impl<T: 'static + Send + Sync + Debug + Clone + Default, const N: usize> SimTxBundle<T, N> {
    // ss[related philosophy.structural-hierarchy]
    fn new(tx_bundle: Arc<[SteadyTx<T>; N]>) -> Self {
        SimTxBundle { tx_bundle, index: 0 }
    }
}

// ss[related testing.sim-producer-close]
impl<C: SteadyActor, T: 'static + Send + Sync + Debug + Clone + Default, const N: usize> SimRunner<C>
    for SimTxBundle<T, N>
{
    // ss[related testing.sim-producer-close]
    fn step(&mut self) -> Result<SimStepResult, Box<dyn Error>> {
        let i = self.index % N;
        let tx = &self.tx_bundle[i];
        self.index += 1;
        if let Some(mut guard) = tx.try_lock() {
            if !guard.shared_is_full() {
                let dummy = T::default();
                let _ = guard.shared_try_send(dummy);
                Ok(SimStepResult::DidWork)
            } else {
                Ok(SimStepResult::NoWork)
            }
        } else {
            Ok(SimStepResult::NoWork)
        }
    }

    // ss[related testing.sim-producer-close]
    fn stage_step(
        &mut self,
        actor: &mut C,
        responder: &SideChannelResponder,
    ) -> Result<SimStepResult, Box<dyn Error>> {
        for lane in 0..N {
            if let Some(mut guard) = self.tx_bundle[lane].try_lock() {
                let r = responder.simulate_direction(&mut *guard, actor, lane)?;
                if r == SimStepResult::DidWork {
                    return Ok(SimStepResult::DidWork);
                }
            }
        }
        Ok(SimStepResult::NoWork)
    }

    // ss[related testing.sim-producer-close]
    fn close_outputs_on_simulated_stop(&mut self) -> Result<(), Box<dyn Error>> {
        for lane in 0..N {
            let tx = &self.tx_bundle[lane];
            core_exec::block_on(async {
                let mut g = tx.lock().await;
                g.mark_closed();
            });
        }
        Ok(())
    }
}

/// Minimal [`SteadyActor`] for simulation and unit tests.
// ss[related testing.sim-producer-close]
pub struct TestActor {
    /// Optional side-channel responder for control-plane tests.
    pub sidechannel: Option<SideChannelResponder>,
    /// When false, the actor reports shutdown to the graph liveliness layer.
    pub is_running: bool,
}

// ss[related testing.sim-producer-close]
impl TestActor {
    /// Creates a running test actor with no side channel attached.
    // ss[related philosophy.structural-hierarchy]
    pub fn new() -> Self {
        TestActor {
            sidechannel: None,
            is_running: true,
        }
    }
}

// ss[related testing.sim-producer-close]
impl SteadyActor for TestActor {
    // ss[related philosophy.structural-hierarchy]
    fn frame_rate_ms(&self) -> u64 { 100 }
    // ss[related philosophy.structural-hierarchy]
    fn regeneration(&self) -> u32 { 0 }
    // ss[related testing.sim-producer-close]
    fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> { None }
    // ss[related philosophy.structural-hierarchy]
    async fn simulated_behavior(self, _sims: Vec<&dyn IntoSimRunner<Self>>) -> Result<(), Box<dyn Error>> { Ok(()) }
    // ss[related philosophy.structural-hierarchy]
    fn loglevel(&self, _loglevel: crate::LogLevel) {}
    // ss[related testing.sim-producer-close]
    fn relay_stats_smartly(&mut self) -> bool { false }
    // ss[related philosophy.structural-hierarchy]
    fn relay_stats(&mut self) {}
    // ss[related philosophy.structural-hierarchy]
    async fn relay_stats_periodic(&mut self, _duration_rate: Duration) -> bool { true }
    // ss[related testing.sim-producer-close]
    fn is_liveliness_in(&self, _target: &[GraphLivelinessState]) -> bool { false }
    // ss[related philosophy.structural-hierarchy]
    fn is_liveliness_building(&self) -> bool { false }
    // ss[related philosophy.structural-hierarchy]
    fn is_liveliness_running(&self) -> bool { false }
    // ss[related testing.sim-producer-close]
    fn is_liveliness_stop_requested(&self) -> bool { false }
    // ss[related philosophy.structural-hierarchy]
    fn is_liveliness_shutdown_timeout(&self) -> Option<Duration> { None }
    // ss[related philosophy.structural-hierarchy]
    fn flush_defrag_messages<S: StreamControlItem>(
        &mut self,
        _item: &mut Tx<S>,
        _data: &mut Tx<u8>,
        _defrag: &mut Defrag<S>,
    ) -> (u32, u32, Option<i32>) { (0, 0, None) }
    // ss[related testing.sim-producer-close]
    async fn wait_periodic(&self, _duration_rate: Duration) -> bool { true }
    // ss[related philosophy.structural-hierarchy]
    async fn wait_timeout(&self, _timeout: Duration) -> bool { true }
    // ss[related philosophy.structural-hierarchy]
    async fn wait(&self, _duration: Duration) {}
    // ss[related testing.sim-producer-close]
    async fn wait_avail<T: RxCore>(&self, _this: &mut T, _size: usize) -> bool { true }
    // ss[related philosophy.structural-hierarchy]
    async fn wait_avail_bundle<T: RxCore>(&self, _this: &mut RxCoreBundle<'_, T>, _size: usize, _ready_channels: usize) -> bool { true }
    // ss[related philosophy.structural-hierarchy]
    async fn wait_avail_index<T: RxCore>(&self, _this: &mut RxCoreBundle<'_, T>, _counts: &[usize]) -> Option<usize> { None }
    // ss[related testing.sim-producer-close]
    async fn wait_future_void<F>(&self, _fut: F) -> bool where F: FusedFuture<Output = ()> + 'static + Send + Sync { true }
    // ss[related philosophy.structural-hierarchy]
    async fn wait_vacant<T: TxCore>(&self, _this: &mut T, _count: T::MsgSize) -> bool { true }
    // ss[related philosophy.structural-hierarchy]
    async fn wait_vacant_bundle<T: TxCore>(&self, _this: &mut TxCoreBundle<'_, T>, _count: T::MsgSize, _ready_channels: usize) -> bool { true }
    // ss[related testing.sim-producer-close]
    async fn wait_vacant_index<T: TxCore>(&self, _this: &mut TxCoreBundle<'_, T>, _counts: &[T::MsgSize]) -> Option<usize> { None }
    // ss[related philosophy.structural-hierarchy]
    async fn wait_avail_vacant_index<R: RxCore, T: TxCore>(
        &self,
        _rx: &mut RxCoreBundle<'_, R>,
        _tx: &mut TxCoreBundle<'_, T>,
        _avail_counts: &[usize],
        _vacant_counts: &[T::MsgSize],
    ) -> Option<usize> { None }
    // ss[related testing.sim-producer-close]
    async fn wait_shutdown(&self) -> bool { true }
    // ss[related philosophy.structural-hierarchy]
    fn peek_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceSource<'b> where T: RxCore { unimplemented!() }
    // ss[related philosophy.structural-hierarchy]
    fn advance_take_index<T: RxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> RxDone { unimplemented!() }
    // ss[related testing.sim-producer-close]
    fn take_slice<T: RxCore>(&mut self, _this: &mut T, _target: T::SliceTarget<'_>) -> RxDone where T::MsgItem: Copy { unimplemented!() }
    // ss[related philosophy.structural-hierarchy]
    fn send_slice<T: TxCore>(&mut self, _this: &mut T, _source: T::SliceSource<'_>) -> TxDone where T::MsgOut: Copy { unimplemented!() }
    // ss[related philosophy.structural-hierarchy]
    fn poke_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceTarget<'b> where T: TxCore { unimplemented!() }
    // ss[related testing.sim-producer-close]
    fn advance_send_index<T: TxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> TxDone { unimplemented!() }
    // ss[related philosophy.structural-hierarchy]
    fn try_peek<'a, T>(&'a self, _this: &'a mut Rx<T>) -> Option<&'a T> { None }
    // ss[related philosophy.structural-hierarchy]
    fn try_peek_iter<'a, T>(&'a self, _this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a { std::iter::empty() }
    // ss[related testing.sim-producer-close]
    fn is_empty<T: RxCore>(&self, _this: &mut T) -> bool { true }
    // ss[related philosophy.structural-hierarchy]
    fn avail_units<T: RxCore>(&self, _this: &mut T) -> T::MsgSize { unimplemented!() }
    // ss[related philosophy.structural-hierarchy]
    async fn peek_async<'a, T: RxCore>(&'a self, _this: &'a mut T) -> Option<T::MsgPeek<'a>> { None }
    // ss[related testing.sim-producer-close]
    fn send_iter_until_full<T, I: Iterator<Item = T>>(&mut self, _this: &mut Tx<T>, _iter: I) -> usize { 0 }
    // ss[related philosophy.structural-hierarchy]
    fn try_send<T: TxCore>(&mut self, this: &mut T, msg: T::MsgIn<'_>) -> SendOutcome<T::MsgOut> {
        SteadyActorCore::try_send(this, msg)
    }
    // ss[related testing.sim-producer-close]
    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> {
        SteadyActorCore::try_take(this)
    }
    // ss[related testing.sim-producer-close]
    fn is_full<T: TxCore>(&self, _this: &mut T) -> bool { false }
    // ss[related philosophy.structural-hierarchy]
    fn vacant_units<T: TxCore>(&self, _this: &mut T) -> T::MsgSize { unimplemented!() }
    // ss[related philosophy.structural-hierarchy]
    async fn wait_empty<T: TxCore>(&self, _this: &mut T) -> bool { true }
    // ss[related testing.sim-producer-close]
    fn take_into_iter<'a, T: Sync + Send>(&mut self, _this: &'a mut Rx<T>) -> impl Iterator<Item = T> + 'a { std::iter::empty() }
    // ss[related philosophy.structural-hierarchy]
    async fn call_async<F>(&self, _operation: F) -> Option<F::Output> where F: Future { None }
    // ss[related philosophy.structural-hierarchy]
    fn call_blocking<F, T>(&self, f: F) -> BlockingCallFuture<T> where F: FnOnce() -> T + Send + 'static, T: Send + 'static {
        BlockingCallFuture(core_exec::spawn_blocking(f))
    }
    // ss[related testing.sim-producer-close]
    async fn send_async<T: TxCore>(&mut self, _this: &mut T, _a: T::MsgIn<'_>, _saturation: SendSaturation) -> SendOutcome<T::MsgOut> { SendOutcome::Success }
    // ss[related philosophy.structural-hierarchy]
    async fn take_async<T>(&mut self, _this: &mut Rx<T>) -> Option<T> { None }
    // ss[related philosophy.structural-hierarchy]
    async fn take_async_with_timeout<T>(&mut self, _this: &mut Rx<T>, _timeout: Duration) -> Option<T> { None }
    // ss[related testing.sim-producer-close]
    async fn yield_now(&self) { yield_now().await; }
    // ss[related philosophy.structural-hierarchy]
    fn sidechannel_responder(&self) -> Option<SideChannelResponder> { self.sidechannel.clone() }
    // ss[related philosophy.structural-hierarchy]
    fn is_running<F: FnMut() -> bool>(&mut self, _accept_fn: F) -> bool { self.is_running }
    // ss[related testing.sim-producer-close]
    async fn request_shutdown(&mut self) { self.is_running = false; }
    // ss[related philosophy.structural-hierarchy]
    fn args<A: Any>(&self) -> Option<&A> { None }
    // ss[related philosophy.structural-hierarchy]
    fn identity(&self) -> ActorIdentity { ActorIdentity::new(0, "test", None) }
    // ss[related testing.sim-producer-close]
    fn is_showstopper<T>(&self, _rx: &mut Rx<T>, _threshold: usize) -> bool { false }
    // ss[related philosophy.structural-hierarchy]
    fn set_dot_display_text(&mut self, _text: Option<&str>) {}
}

#[cfg(test)]
// ss[related testing.sim-producer-close]
#[path = "simulate_edge_proptest.rs"]
// ss[related philosophy.structural-hierarchy]
mod simulate_edge_proptest;

#[cfg(test)]
// ss[related testing.sim-producer-close]
mod tests {
    // ss[related philosophy.structural-hierarchy]
    use super::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::channel_builder::ChannelBuilder;
    // ss[related testing.sim-producer-close]
    use crate::core_exec;

    // ss[verify testing.stage-manager-integration]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn test_simulate_single_rx() {
        let builder = ChannelBuilder::default().with_capacity(4);
        let (tx, rx) = builder.build_channel::<i32>();
        tx.testing_send_all(vec![10, 20, 30], false);
        let mut runner: Box<dyn SimRunner<TestActor>> = IntoSimRunner::<TestActor>::into_sim_runner(&rx.clone());
        assert_eq!(runner.step().unwrap(), SimStepResult::DidWork);
        assert_eq!(runner.step().unwrap(), SimStepResult::DidWork);
        assert_eq!(runner.step().unwrap(), SimStepResult::DidWork);
        assert_eq!(runner.step().unwrap(), SimStepResult::NoWork);
    }

    // ss[verify testing.stage-manager-integration]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn test_simulate_single_tx() {
        let builder = ChannelBuilder::default().with_capacity(2);
        let (tx, _rx) = builder.build_channel::<i32>();
        let mut runner: Box<dyn SimRunner<TestActor>> = IntoSimRunner::<TestActor>::into_sim_runner(&tx.clone());
        assert_eq!(runner.step().unwrap(), SimStepResult::DidWork);
        assert_eq!(runner.step().unwrap(), SimStepResult::DidWork);
        assert_eq!(runner.step().unwrap(), SimStepResult::NoWork);
    }

    // ss[verify testing.sim-producer-close]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn test_sim_tx_close_outputs_on_simulated_stop() {
        let builder = ChannelBuilder::default().with_capacity(4);
        let (tx, rx) = builder.build_channel::<i32>();
        let mut runner: Box<dyn SimRunner<TestActor>> = IntoSimRunner::<TestActor>::into_sim_runner(&tx.clone());
        runner.close_outputs_on_simulated_stop().unwrap();
        core_exec::block_on(async {
            let rx_est = rx.clone();
            let mut g = rx_est.lock().await;
            assert!(g.is_closed_and_empty());
        });
    }
}
