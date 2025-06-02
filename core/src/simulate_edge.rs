//! The `simulate_edge` module provides support for running simulation runners
//! using a shared command context and side-channel responders. It allows
//! asynchronous execution of senders and receivers in a controlled test or
//! simulation environment, coordinating tasks via a single cooperative loop
//! to prevent deadlocks from multiple concurrent `is_running` loops.

use std::error::Error;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use futures::lock::Mutex as AsyncMutex; // Use futures::lock::Mutex for async compatibility
use crate::{await_for_all, SteadyCommander, SteadyRx, SteadyStreamRx, SteadyStreamTx, SteadyTx, StreamSessionMessage, StreamSimpleMessage};
use crate::graph_testing::SideChannelResponder;
use crate::i;
use log::*;
use crate::core_rx::RxCore;

/// Result of a single simulation step indicating what happened during execution.
#[derive(Debug, Clone, PartialEq)]
pub enum SimStepResult {
    DidWork,    // Performed meaningful work
    NoWork,     // No work available but still active
    Finished,   // Completed all work
}

/// A function type that executes a single step of simulation work.
pub type SimRunner<C: SteadyCommander + 'static> = Box<
    dyn Fn(SideChannelResponder, usize, &mut C) -> Pin<Box<dyn Future<Output = Result<SimStepResult, Box<dyn Error>>>>>
>;

/// Converts components into a simulation runner function.
pub trait IntoSimRunner<C: SteadyCommander + 'static> {
    fn run(&self, responder: SideChannelResponder, index: usize, cmd: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>>;
}

/// Tracks the state of an individual simulation runner for timeout and error handling.
#[derive(Debug)]
struct SimulationState {
    consecutive_no_work_cycles: usize,
    max_no_work_cycles: usize,
    simulation_name: String,
}

impl SimulationState {
    fn new(simulation_name: String, max_no_work_cycles: usize) -> Self {
        Self {
            consecutive_no_work_cycles: 0,
            max_no_work_cycles,
            simulation_name,
        }
    }

    fn record_step_result(&mut self, result: &SimStepResult) -> Result<bool, Box<dyn Error>> {
        match result {
            SimStepResult::DidWork => {
                self.consecutive_no_work_cycles = 0;
                Ok(true)
            }
            SimStepResult::NoWork => {
                self.consecutive_no_work_cycles += 1;
                if self.consecutive_no_work_cycles > self.max_no_work_cycles {
                    Err(format!(
                        "Simulation '{}' exceeded maximum consecutive no-work cycles ({}). \
                         Check test setup for stalled runners.",
                        self.simulation_name, self.max_no_work_cycles
                    ).into())
                } else {
                    Ok(true)
                }
            }
            SimStepResult::Finished => {
                debug!("Simulation '{}' completed successfully", self.simulation_name);
                Ok(false)
            }
        }
    }
}

// --- Simulation Runner Implementations ---

impl<T, C> IntoSimRunner<C> for SteadyRx<T>
where
    T: 'static + Debug + Eq + Send + Sync,
    C: SteadyCommander + 'static,
{
    fn run(&self, responder: SideChannelResponder, index: usize, cmd: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
        let value = this.clone();

                match value.try_lock() {
                    Some(mut guard) => {
                        if cmd.is_running(&mut || guard.is_closed_and_empty()) {
                            responder.simulate_wait_for(&mut guard, cmd, index, run_duration)
                        } else {
                            Ok(SimStepResult::Finished)
                        }
                    }
                    None => Ok(SimStepResult::NoWork),
                }


    }
}

impl<C> IntoSimRunner<C> for SteadyStreamRx<StreamSessionMessage>
where
    C: SteadyCommander + 'static,
{
    fn run(&self, responder: SideChannelResponder, index: usize, cmd: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
                let this = self.clone();
                match this.try_lock() {
                    Some(mut guard) => {
                        if cmd.is_running(&mut || guard.is_closed_and_empty()) {
                            responder.simulate_wait_for(&mut guard, cmd, index, run_duration)
                        } else {
                            Ok(SimStepResult::Finished)
                        }
                    }
                    None => Ok(SimStepResult::NoWork),
                }

    }
}

impl<C> IntoSimRunner<C> for SteadyStreamRx<StreamSimpleMessage>
where
    C: SteadyCommander + 'static,
{
    fn run(&self, responder: SideChannelResponder, index: usize, cmd: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
                match this.try_lock() {
                    Some(mut guard) => {
                        if cmd.is_running(&mut || guard.is_closed_and_empty()) {
                            responder.simulate_wait_for(&mut guard, cmd, index, run_duration)
                        } else {
                            Ok(SimStepResult::Finished)
                        }
                    }
                    None => Ok(SimStepResult::NoWork),
                }

    }
}

impl<T, C> IntoSimRunner<C> for SteadyTx<T>
where
    T: 'static + Debug + Clone + Send + Sync,
    C: SteadyCommander + 'static,
{
    fn run(&self, responder: SideChannelResponder, index: usize, cmd: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
                match this.try_lock() {
                    Some(mut guard) => {
                        if cmd.is_running(&mut || guard.mark_closed()) {
                            responder.simulate_direction(&mut guard, cmd, index)
                        } else {
                            Ok(SimStepResult::Finished)
                        }
                    }

                    None => Ok(SimStepResult::NoWork),
                }

    }
}

impl<C> IntoSimRunner<C> for SteadyStreamTx<StreamSessionMessage>
where
    C: SteadyCommander + 'static,
{
    fn run(&self, responder: SideChannelResponder, index: usize, cmd: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
                match this.try_lock() {
                    Some(mut guard) => {
                        if cmd.is_running(&mut || guard.mark_closed()) {
                            responder.simulate_direction(&mut guard, cmd, index)
                        } else {
                            Ok(SimStepResult::Finished)
                        }
                    }
                    None => Ok(SimStepResult::NoWork),
                }
    }
}

impl<C> IntoSimRunner<C> for SteadyStreamTx<StreamSimpleMessage>
where
    C: SteadyCommander + 'static,
{
    fn run(&self, responder: SideChannelResponder, index: usize, cmd: &mut C, run_duration: Duration) -> Result<SimStepResult, Box<dyn Error>> {
        let this = self.clone();
                match this.try_lock() {
                    Some(mut guard) => {
                        if cmd.is_running(&mut || guard.mark_closed()) {
                            responder.simulate_direction(&mut guard, cmd, index)
                        } else {
                            Ok(SimStepResult::Finished)
                        }
                    }
                    None => Ok(SimStepResult::NoWork),
                }

    }
}

// --- Main Simulation Function ---

/// Executes multiple simulation runners using a single cooperative scheduling loop.
///
/// Uses round-robin scheduling to ensure fairness, with per-runner state tracking
/// for timeouts and detailed error reporting.
pub(crate) async fn simulated_behavior<C: SteadyCommander + 'static>(
    cmd: &mut C,
    sims: Vec<&dyn IntoSimRunner<C>>,
) -> Result<(), Box<dyn Error>> {
    let responders = cmd.sidechannel_responder().ok_or("No responder")?;

    let mut simulation_states: Vec<SimulationState> = (0..sims.len())
        .map(|i| SimulationState::new(format!("sim_{}", i), 10000))
        .collect();

    let mut active_simulations: Vec<bool> = vec![true; sims.len()];
    let mut active_count = sims.len();

    info!("Starting simulation with {} runners for {:?}", sims.len(), cmd.identity());
    let now = Instant::now();

    //NOTE: each runner detects shutdown and closes its outgoign connections as needed.
    while cmd.is_running(&mut || 0==active_count) {
        let mut any_work_done = false;

        //WARNING: this is a tight loop over all the simulators, as a result we must
        //         limit our selves the frame rate which is also helpful to capture data
        //         as it happens in order for debugging later.
        await_for_all!(cmd.wait_periodic(Duration::from_millis(cmd.frame_rate_ms())));

        for (index, sim) in sims.iter().enumerate() {
            if !active_simulations[index] {
                continue;
            }

            let run_duration = now.elapsed();

            match sim.run(responders.clone(), index, cmd, run_duration) {
                Ok(step_result) => {
                    if matches!(step_result, SimStepResult::DidWork) {
                        any_work_done = true;
                    }
                    match simulation_states[index].record_step_result(&step_result) {
                        Ok(true) => {}
                        Ok(false) => {
                            active_simulations[index] = false;
                            active_count = active_count.saturating_sub(1); // Prevent underflow
                            info!("Simulation {} completed", index);
                        }
                        Err(timeout_error) => {
                            error!("Simulation {} timed out: {}", index, timeout_error);
                            cmd.request_shutdown().await;
                            return Err(timeout_error);
                        }
                    }
                }
                Err(sim_error) => {
                    error!("Simulation {} failed: {}", index, sim_error);
                    active_simulations[index] = false;
                    active_count = active_count.saturating_sub(1); // Prevent underflow
                    cmd.request_shutdown().await;
                    return Err(format!("Simulation {} failed: {}", index, sim_error).into());
                }
            }
        }

        if !any_work_done && active_count > 0 {
            trace!("No work done this cycle, continuing...");
        }
    }

    if active_count > 0 {
        warn!("Exiting with {} active simulations due to shutdown", active_count);
    } else {
        info!("All simulations completed successfully");
    }

    Ok(())
}