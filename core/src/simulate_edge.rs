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
