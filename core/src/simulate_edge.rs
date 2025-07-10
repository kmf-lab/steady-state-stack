//! The `simulate_edge` module provides support for running simulation runners
//! using a shared command context and side-channel responders. It allows
//! asynchronous execution of senders and receivers in a controlled test or
//! simulation environment, coordinating tasks via a single cooperative loop
//! to prevent deadlocks from multiple concurrent `is_running` loops.

use std::error::Error;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};
use crate::{SteadyActor, SteadyRx, SteadyStreamRx, SteadyStreamTx, SteadyTx, StreamIngress, StreamEgress};
use crate::graph_testing::SideChannelResponder;
use crate::i;
use log::*;

/// Represents the outcome of a single step in a simulation, indicating the nature
/// of the work performed during that step.
#[derive(Debug, Clone, PartialEq)]
pub enum SimStepResult {
    /// Indicates that the simulation step performed meaningful work, such as
    /// processing data or advancing the simulation state.
    DidWork,
    /// Indicates that no work was available to perform, but the simulation is
    /// still active and may have work in future steps.
    NoWork,
    /// Indicates that the simulation has completed all its work and will not
    /// produce further results.
    Finished,
}

/// Defines a function type for executing a single step of simulation work.
/// This is a boxed dynamic function that takes a side-channel responder, an
/// index, and a mutable actor, returning a pinned boxed future that resolves
/// to a result containing either a simulation step outcome or an error.
pub type SimRunner = Box<
    dyn Fn(SideChannelResponder, usize, &mut dyn SteadyActor) -> Pin<Box<dyn Future<Output = Result<SimStepResult, Box<dyn Error>>>>>
>;

/// A trait for converting components into a simulation runner function.
/// Implementors of this trait define how a component executes a single step
/// of simulation work, interacting with a responder, actor, and simulation
/// parameters.
pub trait IntoSimRunner<C: SteadyActor + 'static> {
    /// Executes a single simulation step for the given component, using the
    /// provided responder, index, actor, and run duration. Returns a result
    /// indicating the outcome of the step or an error if the step failed.
    fn run(&self, responder: &SideChannelResponder, index: usize, actor: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>>;
}

/// Maintains the state of an individual simulation runner, tracking metrics
/// such as consecutive cycles with no work and the simulation's name for
/// logging and error reporting purposes.
#[derive(Debug)]
struct SimulationState {
    /// Counts the number of consecutive simulation steps where no work was
    /// performed, used to detect potential stalls or timeouts.
    consecutive_no_work_cycles: u64,
    /// A unique identifier for the simulation, used in logging and error
    /// messages to distinguish between multiple runners.
    simulation_name: String,
}

impl SimulationState {
    /// Creates a new simulation state with the specified name and initializes
    /// the no-work cycle counter to zero.
    fn new(simulation_name: String) -> Self {
        Self {
            consecutive_no_work_cycles: 0,
            simulation_name,
        }
    }

    /// Updates the simulation state based on the result of a simulation step
    /// and determines whether the simulation should continue. Returns a result
    /// indicating whether the simulation is still active (true) or has finished
    /// (false), or an error if the step result processing failed.
    fn record_step_result(&mut self, result: &SimStepResult) -> Result<bool, Box<dyn Error>> {
        match result {
            SimStepResult::DidWork => {
                // Reset the no-work counter since work was performed.
                self.consecutive_no_work_cycles = 0;
                Ok(true)
            }
            SimStepResult::NoWork => {
                // Increment the no-work counter to track potential stalls.
                self.consecutive_no_work_cycles += 1;
                Ok(true)
            }
            SimStepResult::Finished => {
                // Log completion and indicate the simulation is no longer active.
                trace!("Simulation '{}' completed successfully", self.simulation_name);
                Ok(false)
            }
        }
    }
}

// --- Simulation Runner Implementations ---

/// Implementation of the `IntoSimRunner` trait for a steady receiver channel.
/// This allows a receiver to participate in a simulation by processing incoming
/// data or waiting for new data to arrive.
impl<T, C> IntoSimRunner<C> for SteadyRx<T>
where
    T: 'static + Debug + Eq + Send + Sync,
    C: SteadyActor + 'static,
{
    /// Executes a single simulation step for the receiver. Attempts to acquire
    /// a lock on the receiver and checks if the actor is still running and if
    /// the channel is open. If active, it simulates waiting for data using the
    /// responder. Returns a result indicating the step's outcome or an error if
    /// the operation failed.
    fn run(&self, responder: &SideChannelResponder, index: usize, actor: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
        let value = this.clone();
        match value.try_lock() {
            Some(mut guard) => {
                if actor.is_running(&mut || i!(0 == responder.avail()) && i!(guard.is_closed())) {
                    // Simulate waiting for data if the channel is active.
                    responder.simulate_wait_for(&mut guard, actor, index, run_duration)
                } else {
                    // Indicate completion if the channel is closed or the actor is shutting down.
                    Ok(SimStepResult::Finished)
                }
            }
            None => {
                // Indicate no work was done if the lock could not be acquired.
                Ok(SimStepResult::NoWork)
            }
        }
    }
}

/// Implementation of the `IntoSimRunner` trait for a steady stream receiver
/// handling ingress streams. This enables the receiver to process streaming
/// data in a simulation environment.
impl<C> IntoSimRunner<C> for SteadyStreamRx<StreamIngress>
where
    C: SteadyActor + 'static,
{
    /// Executes a single simulation step for the ingress stream receiver.
    /// Attempts to acquire a lock on the receiver and checks if the actor is
    /// running and the channel is open. If active, it simulates waiting for
    /// stream data using the responder. Returns a result indicating the step's
    /// outcome or an error if the operation failed.
    fn run(&self, responder: &SideChannelResponder, index: usize, actor: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
        match this.try_lock() {
            Some(mut guard) => {
                if actor.is_running(&mut || i!(0 == responder.avail()) && i!(guard.is_closed())) {
                    // Simulate waiting for stream data if the channel is active.
                    responder.simulate_wait_for(&mut guard, actor, index, run_duration)
                } else {
                    // Indicate completion if the channel is closed or the actor is shutting down.
                    Ok(SimStepResult::Finished)
                }
            }
            None => {
                // Indicate no work was done if the lock could not be acquired.
                Ok(SimStepResult::NoWork)
            }
        }
    }
}

/// Implementation of the `IntoSimRunner` trait for a steady stream receiver
/// handling egress streams. This enables the receiver to process streaming
/// data in a simulation environment.
impl<C> IntoSimRunner<C> for SteadyStreamRx<StreamEgress>
where
    C: SteadyActor + 'static,
{
    /// Executes a single simulation step for the egress stream receiver.
    /// Attempts to acquire a lock on the receiver and checks if the actor is
    /// running and the channel is open. If active, it simulates waiting for
    /// stream data using the responder. Returns a result indicating the step's
    /// outcome or an error if the operation failed.
    fn run(&self, responder: &SideChannelResponder, index: usize, actor: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
        match this.try_lock() {
            Some(mut guard) => {
                if actor.is_running(&mut || i!(0 == responder.avail()) && i!(guard.is_closed())) {
                    // Simulate waiting for stream data if the channel is active.
                    responder.simulate_wait_for(&mut guard, actor, index, run_duration)
                } else {
                    // Indicate completion if the channel is closed or the actor is shutting down.
                    Ok(SimStepResult::Finished)
                }
            }
            None => {
                // Indicate no work was done if the lock could not be acquired.
                Ok(SimStepResult::NoWork)
            }
        }
    }
}

/// Implementation of the `IntoSimRunner` trait for a steady transmitter channel.
/// This allows a transmitter to participate in a simulation by sending data or
/// marking the channel as closed.
impl<T, C> IntoSimRunner<C> for SteadyTx<T>
where
    T: 'static + Debug + Clone + Send + Sync,
    C: SteadyActor + 'static,
{
    /// Executes a single simulation step for the transmitter. Attempts to
    /// acquire a lock on the transmitter and checks if the actor is running.
    /// If active, it simulates sending data or closing the channel using the
    /// responder. Returns a result indicating the step's outcome or an error
    /// if the operation failed.
    fn run(&self, responder: &SideChannelResponder, index: usize, actor: &mut C, _: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
        match this.try_lock() {
            Some(mut guard) => {
                if actor.is_running(&mut || guard.mark_closed()) {
                    // Simulate sending data or closing the channel if active.
                    responder.simulate_direction(&mut guard, actor, index)
                } else {
                    // Indicate completion if the channel is closed or the actor is shutting down.
                    Ok(SimStepResult::Finished)
                }
            }
            None => {
                // Indicate no work was done if the lock could not be acquired.
                Ok(SimStepResult::NoWork)
            }
        }
    }
}

/// Implementation of the `IntoSimRunner` trait for a steady stream transmitter
/// handling ingress streams. This enables the transmitter to send streaming
/// data in a simulation environment.
impl<C> IntoSimRunner<C> for SteadyStreamTx<StreamIngress>
where
    C: SteadyActor + 'static,
{
    /// Executes a single simulation step for the ingress stream transmitter.
    /// Attempts to acquire a lock on the transmitter and checks if the actor
    /// is running. If active, it simulates sending stream data or closing the
    /// channel using the responder. Returns a result indicating the step's
    /// outcome or an error if the operation failed.
    fn run(&self, responder: &SideChannelResponder, index: usize, actor: &mut C, _run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
        match this.try_lock() {
            Some(mut guard) => {
                if actor.is_running(&mut || guard.mark_closed()) {
                    // Simulate sending stream data or closing the channel if active.
                    responder.simulate_direction(&mut guard, actor, index)
                } else {
                    // Indicate completion if the channel is closed or the actor is shutting down.
                    Ok(SimStepResult::Finished)
                }
            }
            None => {
                // Indicate no work was done if the lock could not be acquired.
                Ok(SimStepResult::NoWork)
            }
        }
    }
}

/// Implementation of the `IntoSimRunner` trait for a steady stream transmitter
/// handling egress streams. This enables the transmitter to send streaming
/// data in a simulation environment.
impl<C> IntoSimRunner<C> for SteadyStreamTx<StreamEgress>
where
    C: SteadyActor + 'static,
{
    /// Executes a single simulation step for the egress stream transmitter.
    /// Attempts to acquire a lock on the transmitter and checks if the actor
    /// is running. If active, it simulates sending stream data or closing the
    /// channel using the responder. Returns a result indicating the step's
    /// outcome or an error if the operation failed.
    fn run(&self, responder: &SideChannelResponder, index: usize, actor: &mut C, _run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
        match this.try_lock() {
            Some(mut guard) => {
                if actor.is_running(&mut || guard.mark_closed()) {
                    // Simulate sending stream data or closing the channel if active.
                    responder.simulate_direction(&mut guard, actor, index)
                } else {
                    // Indicate completion if the channel is closed or the actor is shutting down.
                    Ok(SimStepResult::Finished)
                }
            }
            None => {
                // Indicate no work was done if the lock could not be acquired.
                Ok(SimStepResult::NoWork)
            }
        }
    }
}

// --- Main Simulation Function ---

/// Executes multiple simulation runners in a single cooperative scheduling loop.
/// This function uses round-robin scheduling to ensure fair execution of all
/// runners, tracks per-runner state for timeouts, and provides detailed error
/// reporting. It continues running until all simulations complete or the actor
/// shuts down.
pub(crate) async fn simulated_behavior<C: SteadyActor + 'static>(
    actor: &mut C,
    sims: Vec<&dyn IntoSimRunner<C>>,
) -> Result<(), Box<dyn Error>> {

    let responder = actor.sidechannel_responder().ok_or("No responder")?;

    let mut simulation_states: Vec<SimulationState> = (0..sims.len())
        .map(|i| SimulationState::new(format!("sim_{}", i)))
        .collect();

    let mut active_simulations: Vec<bool> = vec![true; sims.len()];

    let mut active_count = sims.len();

    trace!("Starting simulation with {} runners for {:?}", sims.len(), actor.identity());
    let now = Instant::now();

    while actor.is_running(&mut || i!(0 == active_count)) {

        let mut any_work_done = false;

        actor.call_async(responder.wait_avail()).await;

        for (index, sim) in sims.iter().enumerate() {
            if !active_simulations[index] {
                continue;
            }

            let run_duration = now.elapsed();

            match sim.run(&responder, index, actor, run_duration) {
                Ok(step_result) => {
                    if matches!(step_result, SimStepResult::DidWork) {
                        any_work_done = true;
                    }
                    match simulation_states[index].record_step_result(&step_result) {
                        Ok(true) => {
                            // Runner is still active, continue to the next.
                        }
                        Ok(false) => {
                            active_simulations[index] = false;
                            active_count = active_count.saturating_sub(1);
                        }
                        Err(timeout_error) => {
                            actor.request_shutdown().await;
                            active_simulations[index] = false;
                            active_count = active_count.saturating_sub(1);
                            error!("Simulation {} timed out: {} new count {}", index, timeout_error, active_count);
                        }
                    }
                }
                Err(sim_error) => {
                    active_simulations[index] = false;
                    actor.request_shutdown().await;
                    return Err(format!("Simulation {} failed: {}", index, sim_error).into());
                }
            }
        }

        if !any_work_done && active_count > 0 {
            trace!("No work done this cycle, continuing...");
        }
    }

    if active_count > 0 {
        trace!("Exiting with {} active simulations due to shutdown", active_count);
    } else {
        trace!("All simulations completed successfully");
    }

    Ok(())
}

#[cfg(test)]
mod simulate_edge_tests {
    use super::*;
    use std::error::Error;
    use std::time::Duration;
    use crate::graph_testing::SideChannelResponder;
    use crate::{GraphLivelinessState, SteadyActor, StreamControlItem, Tx};
    use crate::SteadyRx;
    use crate::SteadyTx;
    use crate::SteadyStreamRx;
    use crate::SteadyStreamTx;
    use crate::StreamIngress;
    use crate::StreamEgress;
    use crate::channel_builder::ChannelBuilder;
    use crate::GraphBuilder;
    use crate::ActorIdentity;
    use futures::channel::oneshot;
    use std::sync::Arc;
    use futures::lock::Mutex;
    use std::fmt::Debug;
    use std::any::Any;
    use aeron::aeron::Aeron;
    use async_ringbuf::AsyncRb;
    use async_ringbuf::traits::Split;
    use futures_util::future::FusedFuture;
    use crate::channel_builder::ChannelBacking;
    use crate::*;
    use crate::distributed::aqueduct_stream::Defrag;

    // Dummy actor for testing
    struct TestActor;

    impl SteadyActor for TestActor {
        fn frame_rate_ms(&self) -> u64 { 100 }
        fn regeneration(&self) -> u32 { 0 }
        fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> { None }
        async fn simulated_behavior(self, _sims: Vec<&dyn IntoSimRunner<Self>>) -> Result<(), Box<dyn Error>> { Ok(()) }
        fn loglevel(&self, _loglevel: crate::LogLevel) {}
        fn relay_stats_smartly(&mut self) -> bool { false }
        fn relay_stats(&mut self) {}
        async fn relay_stats_periodic(&mut self, _duration_rate: Duration) -> bool { false }
        fn is_liveliness_in(&self, _target: &[GraphLivelinessState]) -> bool { true }
        fn is_liveliness_building(&self) -> bool { false }
        fn is_liveliness_running(&self) -> bool { true }
        fn is_liveliness_stop_requested(&self) -> bool { false }
        fn is_liveliness_shutdown_timeout(&self) -> Option<Duration> { None }
        fn flush_defrag_messages<S: StreamControlItem>(
            &mut self,
            _item: &mut Tx<S>,
            _data: &mut Tx<u8>,
            _defrag: &mut Defrag<S>,
        ) -> (u32, u32, Option<i32>) { (0, 0, None) }
        async fn wait_periodic(&self, _duration_rate: Duration) -> bool { true }
        async fn wait_timeout(&self, _timeout: Duration) -> bool { true }
        async fn wait(&self, _duration: Duration) {}
        async fn wait_avail<T: RxCore>(&self, _this: &mut T, _size: usize) -> bool { true }
        async fn wait_avail_bundle<T: RxCore>(
            &self,
            _this: &mut RxCoreBundle<'_, T>,
            _size: usize,
            _ready_channels: usize,
        ) -> bool { true }
        async fn wait_future_void<F>(&self, _fut: F) -> bool where F: FusedFuture<Output = ()> + 'static + Send + Sync { true }
        async fn wait_vacant<T: TxCore>(&self, _this: &mut T, _count: T::MsgSize) -> bool { true }
        async fn wait_vacant_bundle<T: TxCore>(
            &self,
            _this: &mut TxCoreBundle<'_, T>,
            _count: T::MsgSize,
            _ready_channels: usize,
        ) -> bool { true }
        async fn wait_shutdown(&self) -> bool { false }
        fn peek_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceSource<'b> where T: RxCore { unimplemented!() }
        fn advance_take_index<T: RxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> RxDone { unimplemented!() }
        fn take_slice<T: RxCore>(
            &mut self,
            _this: &mut T,
            _target: T::SliceTarget<'_>,
        ) -> RxDone where T::MsgItem: Copy { unimplemented!() }
        fn send_slice<T: TxCore>(
            &mut self,
            _this: &mut T,
            _source: T::SliceSource<'_>,
        ) -> TxDone where T::MsgOut: Copy { unimplemented!() }
        fn poke_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceTarget<'b> where T: TxCore { unimplemented!() }
        fn advance_send_index<T: TxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> TxDone { unimplemented!() }
        fn try_peek<'a, T>(&'a self, _this: &'a mut Rx<T>) -> Option<&'a T> { None }
        fn try_peek_iter<'a, T>(
            &'a self,
            _this: &'a mut Rx<T>,
        ) -> impl Iterator<Item = &'a T> + 'a { std::iter::empty() }
        fn is_empty<T: RxCore>(&self, _this: &mut T) -> bool { false }
        fn avail_units<T: RxCore>(&self, this: &mut T) -> T::MsgSize { this.one() }
        async fn peek_async<'a, T: RxCore>(
            &'a self,
            _this: &'a mut T,
        ) -> Option<T::MsgPeek<'a>> { None }
        fn send_iter_until_full<T, I: Iterator<Item = T>>(
            &mut self,
            _this: &mut Tx<T>,
            _iter: I,
        ) -> usize { 0 }
        fn try_send<T: TxCore>(
            &mut self,
            this: &mut T,
            msg: T::MsgIn<'_>,
        ) -> SendOutcome<T::MsgOut> { SendOutcome::Success }
        fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> { None }
        fn is_full<T: TxCore>(&self, _this: &mut T) -> bool { false }
        fn vacant_units<T: TxCore>(&self, this: &mut T) -> T::MsgSize { this.one() }
        async fn wait_empty<T: TxCore>(&self, _this: &mut T) -> bool { true }
        fn take_into_iter<'a, T: Sync + Send>(
            &mut self,
            _this: &'a mut Rx<T>,
        ) -> impl Iterator<Item = T> + 'a { std::iter::empty() }
        async fn call_async<F>(&self, _operation: F) -> Option<F::Output> where F: Future { None }
        async fn call_blocking<F, T>(&self, _f: F) -> Option<F::Output> where F: FnOnce() -> T + Send + 'static, T: Send + 'static { None }
        async fn send_async<T: TxCore>(
            &mut self,
            _this: &mut T,
            _a: T::MsgIn<'_>,
            _saturation: SendSaturation,
        ) -> SendOutcome<T::MsgOut> { SendOutcome::Success }
        async fn take_async<T>(&mut self, _this: &mut Rx<T>) -> Option<T> { None }
        async fn take_async_with_timeout<T>(
            &mut self,
            _this: &mut Rx<T>,
            _timeout: Duration,
        ) -> Option<T> { None }
        async fn yield_now(&self) {}
        fn sidechannel_responder(&self) -> Option<SideChannelResponder> {
            let (tx, rx) = oneshot::channel();
            let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(10);
            let (sender_tx, receiver_tx) = rb.split();
            let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(10);
            let (sender_rx, receiver_rx) = rb.split();
            let arc = Arc::new(Mutex::new(((sender_tx, receiver_rx), rx)));
            Some(SideChannelResponder::new(arc, ActorIdentity::default()))
        }
        fn is_running<F: FnMut() -> bool>(&mut self, mut accept_fn: F) -> bool { accept_fn() }
        async fn request_shutdown(&mut self) {}
        fn args<A: Any>(&self) -> Option<&A> { None }
        fn identity(&self) -> ActorIdentity { ActorIdentity::default() }
        fn is_showstopper<T>(&self, _rx: &mut Rx<T>, _threshold: usize) -> bool { false }
    }

    // Helper to create a dummy responder
    fn create_dummy_responder() -> SideChannelResponder {
        let (tx, rx) = oneshot::channel();
        let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(10);
        let (sender_tx, receiver_tx) = rb.split();
        let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(10);
        let (sender_rx, receiver_rx) = rb.split();
        let arc = Arc::new(Mutex::new(((sender_tx, receiver_rx), rx)));
        SideChannelResponder::new(arc, ActorIdentity::default())
    }

    // Helper to create a SteadyRx with some data
    fn create_steady_rx<T: Send + Sync + Debug + Eq + 'static>(data: Vec<T>) -> SteadyRx<T> {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(data.len() + 1).build_channel::<T>();
        let tx = tx.clone();
        let rx = rx.clone();
        for item in data {
            let mut tx_guard = tx.try_lock().expect("");
            tx_guard.shared_try_send(item).expect("Failed to send");
        }
        rx
    }

    // Helper to create an empty SteadyRx
    fn create_empty_steady_rx<T: Send + Sync + Debug + Eq + 'static>() -> SteadyRx<T> {
        let mut graph = GraphBuilder::for_testing().build(());
        let (_, rx) = graph.channel_builder().build_channel::<T>();
        rx.clone()
    }

    // Helper to create a SteadyTx
    fn create_steady_tx<T: Send + Sync + Debug + Clone + 'static>() -> SteadyTx<T> {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _) = graph.channel_builder().build_channel::<T>();
        tx.clone()
    }

    // Helper to create SteadyStreamRx<StreamIngress>
    fn create_steady_stream_rx_ingress() -> SteadyStreamRx<StreamIngress> {
        // Mock or minimal setup for StreamIngress
        unimplemented!("Implement mock for StreamIngress if needed");
    }

    // Similarly for other stream types...

    #[test]
    fn test_sim_step_result() {
        assert_eq!(SimStepResult::DidWork, SimStepResult::DidWork);
        assert_eq!(SimStepResult::NoWork, SimStepResult::NoWork);
        assert_eq!(SimStepResult::Finished, SimStepResult::Finished);
        assert_ne!(SimStepResult::DidWork, SimStepResult::NoWork);
    }

    #[test]
    fn test_simulation_state_new() {
        let state = SimulationState::new("test_sim".to_string());
        assert_eq!(state.consecutive_no_work_cycles, 0);
        assert_eq!(state.simulation_name, "test_sim");
    }

    #[test]
    fn test_simulation_state_record_step_result() -> Result<(), Box<dyn Error>> {
        let mut state = SimulationState::new("test".to_string());

        // DidWork
        assert_eq!(state.record_step_result(&SimStepResult::DidWork)?, true);
        assert_eq!(state.consecutive_no_work_cycles, 0);

        // NoWork
        assert_eq!(state.record_step_result(&SimStepResult::NoWork)?, true);
        assert_eq!(state.consecutive_no_work_cycles, 1);

        // Finished
        assert_eq!(state.record_step_result(&SimStepResult::Finished)?, false);

        Ok(())
    }

    #[async_std::test]
    async fn test_into_sim_runner_steady_rx() -> Result<(), Box<dyn Error>> {
        let rx = create_steady_rx(vec![1i32, 2, 3]);
        let responder = create_dummy_responder();
        let mut actor = TestActor;
        let result = rx.run(&responder, 0, &mut actor, Duration::from_secs(1))?;
        assert_eq!(result, SimStepResult::DidWork); // Assuming simulate_wait_for does work
        Ok(())
    }

    #[async_std::test]
    async fn test_into_sim_runner_steady_rx_empty() -> Result<(), Box<dyn Error>> {
        let rx = create_empty_steady_rx::<i32>();
        let responder = create_dummy_responder();
        let mut actor = TestActor;
        let result = rx.run(&responder, 0, &mut actor, Duration::from_secs(1))?;
        // Depending on implementation; adjust based on expected behavior
        Ok(())
    }

    // Add similar tests for other IntoSimRunner implementations
    // For SteadyStreamRx<StreamIngress>, SteadyStreamRx<StreamEgress>, etc.
    // Mock StreamIngress/Egress as needed.

    #[async_std::test]
    async fn test_into_sim_runner_steady_tx() -> Result<(), Box<dyn Error>> {
        let tx = create_steady_tx::<i32>();
        let responder = create_dummy_responder();
        let mut actor = TestActor;
        let result = tx.run(&responder, 0, &mut actor, Duration::from_secs(1))?;
        // Adjust assertion based on simulate_direction behavior
        Ok(())
    }

    #[async_std::test]
    async fn test_simulated_behavior() -> Result<(), Box<dyn Error>> {
        let mut actor = TestActor;
        let rx = create_steady_rx(vec![1i32]);
        let tx = create_steady_tx::<i32>();
        let sims: Vec<&dyn IntoSimRunner<TestActor>> = vec![&rx, &tx];
        let result = simulated_behavior(&mut actor, sims).await;
        assert!(result.is_ok());
        Ok(())
    }



    // Add more tests for coverage, e.g., with multiple sims, error cases, timeouts in record_step_result if applicable.
}