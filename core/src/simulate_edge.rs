//! The `simulate_edge` module provides support for running simulation runners
//! using a shared command context and side-channel responders. It allows
//! asynchronous execution of senders and receivers in a controlled test or
//! simulation environment, coordinating tasks via futures.
use std::error::Error;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use futures_util::future::join_all;
use futures_util::lock::Mutex;
use crate::{core_rx, yield_now, Rx, SteadyCommander, SteadyRx, SteadyStreamRx, SteadyStreamTx, SteadyTx, StreamRx, StreamRxBundleTrait, StreamSessionMessage, StreamSimpleMessage, StreamTx, Tx};
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::distributed::distributed_stream::StreamItem;
use crate::graph_testing::{SideChannelResponder, StageDirection};
use crate::i;
use log::*;

/// A function type that creates and runs simulation tasks.
///
/// `SimRunner<C>` is a boxed function that, when invoked, receives:
/// - an `Arc<Mutex<C>>` command context,
/// - a `SideChannelResponder` for simulating channel events,
/// - an index identifying the simulation task,
/// and returns a pinned boxed future (`Pin<Box<dyn Future<Output = ()>>>`) that
/// encapsulates the asynchronous simulation behavior.
pub type SimRunner<C> = Box<
    dyn Fn(Arc<Mutex<C>>, SideChannelResponder, usize) -> Pin<Box<dyn Future<Output = ()>>>
>;

/// Converts components into a simulation runner function.
///
/// The `IntoSimRunner` trait defines how a type can produce a `SimRunner`,
/// enabling simulation behavior for transmitter (`SimTx`) and receiver (`SimRx`) cores.
pub trait IntoSimRunner<C: SteadyCommander + 'static> {
    /// Transforms `self` into a `SimRunner<C>` that can execute simulation logic.
    fn into_sim_runner(&self) -> SimRunner<C>;
}



impl<T, C> IntoSimRunner<C> for SteadyRx<T>
where
    T: 'static + Debug + Eq + Send + Sync,
    C: SteadyCommander + 'static,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        let this = self.clone();
        Box::new(move |cmd_mutex, responder, index| {
            Box::pin(
                {
                    let value = this.clone();
                    async move {
                        let mut that = value.lock().await;
                        let mut cycles_of_no_work = 0;

                        while cmd_mutex.lock().await.is_running(&mut || i!(that.is_closed_and_empty())) {
                            match responder.simulate_wait_for(&mut that, &cmd_mutex, index).await  {
                                Ok(true) => {
                                    cycles_of_no_work = 0;
                                },
                                Ok(false) => {
                                    cycles_of_no_work += 1;
                                    //TODO: based on timeout shutdown and report..
                                    if cycles_of_no_work > 10000 {
                                        cmd_mutex.lock().await.request_shutdown().await;
                                        error!("stopped on cycle of no work");// TODO: refine this.
                                    }
                                },
                                Err(e) => {
                                    cmd_mutex.lock().await.request_shutdown().await;
                                    error!("Internal Error: {:?}",e);
                                }
                            };

                        }
                    }
                }
            )
        })
    }
}

impl<C> IntoSimRunner<C> for SteadyStreamRx<StreamSessionMessage>
where
    C: SteadyCommander + 'static,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        let this = self.clone();
        Box::new(move |cmd_mutex, responder, index| {
            Box::pin(
                {
                    let value = this.clone();
                    async move {
                        let mut that = value.lock().await;
                        let mut cycles_of_no_work = 0;

                        while cmd_mutex.lock().await.is_running(&mut || i!(that.is_closed_and_empty())) {
                            match responder.simulate_wait_for(&mut that, &cmd_mutex, index).await {
                                Ok(true) => {
                                    cycles_of_no_work = 0;
                                },
                                Ok(false) => {
                                    cycles_of_no_work += 1;
                                    //TODO: based on timeout shutdown and report..
                                    if cycles_of_no_work > 10000 {
                                        cmd_mutex.lock().await.request_shutdown().await;
                                        error!("stopped on cycle of no work");// TODO: refine this.
                                    }
                                },
                                Err(e) => {
                                    cmd_mutex.lock().await.request_shutdown().await;
                                    error!("Internal Error: {:?}",e);
                                }
                            };
                        }
                    }
                }
            )
        })
    }
}

impl<C> IntoSimRunner<C> for SteadyStreamRx<StreamSimpleMessage>
where
    C: SteadyCommander + 'static,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        let this = self.clone();
        Box::new(move |cmd_mutex, responder, index| {
            Box::pin(
                {
                    let value = this.clone();
                    async move {
                        let mut that = value.lock().await;
                        let mut cycles_of_no_work = 0;

                        while cmd_mutex.lock().await.is_running(&mut || i!(that.is_closed_and_empty())) {
                            match responder.simulate_wait_for(&mut that, &cmd_mutex, index).await {
                                Ok(true) => {
                                    cycles_of_no_work = 0;
                                },
                                Ok(false) => {
                                    cycles_of_no_work += 1;
                                    //TODO: based on timeout shutdown and report..
                                    if cycles_of_no_work > 10000 {
                                        cmd_mutex.lock().await.request_shutdown().await;
                                        error!("stopped on cycle of no work");// TODO: refine this.
                                    }
                                },
                                Err(e) => {
                                    cmd_mutex.lock().await.request_shutdown().await;
                                    error!("Internal Error: {:?}",e);
                                }
                            };
                        }
                    }
                }
            )
        })
    }
}



impl<T, C> IntoSimRunner<C> for SteadyTx<T>
where
    T: 'static + Debug + Clone + Send + Sync,
    C: SteadyCommander + 'static,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        let this = self.clone();
        Box::new(move |cmd_mutex, responder, index| {
            Box::pin(
                {
                    let value = this.clone();
                    async move {
                        let mut that = value.lock().await;
                        let mut cycles_of_no_work = 0;
                        while cmd_mutex.lock().await.is_running(&mut || i!(that.mark_closed())) {
                            //   Ok(true)  //did something keep going
                            //   Ok(false) //did nothing becuase it does not pertain - if we are here beyond timeout we must trigger shutdown
                            //   Err("")  // exit now failure, shutdown.

                            match responder.simulate_direction(&mut that, &cmd_mutex, index).await {
                                Ok(true) => {
                                    cycles_of_no_work = 0;
                                },
                                Ok(false) => {
                                    cycles_of_no_work += 1;
                                    //TODO: based on timeout shutdown and report..
                                    if cycles_of_no_work > 10000 {
                                        cmd_mutex.lock().await.request_shutdown().await;
                                        error!("stopped on cycle of no work");// TODO: refine this.
                                    }
                                },
                                Err(e) => {
                                    cmd_mutex.lock().await.request_shutdown().await;
                                    error!("Internal Error: {:?}",e);
                                }
                            };
                        }
                    }
                }
            )
        })
    }
}

impl<C> IntoSimRunner<C> for SteadyStreamTx<StreamSessionMessage>
where
    C: SteadyCommander + 'static,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        let this = self.clone();
        Box::new(move |cmd_mutex, responder, index| {
            Box::pin(
                {
                    let value = this.clone();
                    async move {
                        let mut that = value.lock().await;
                        let mut cycles_of_no_work = 0;
                        while cmd_mutex.lock().await.is_running(&mut || !(that.mark_closed())) {
                            match responder.simulate_direction(&mut that, &cmd_mutex, index).await {
                                Ok(true) => {
                                    cycles_of_no_work = 0;
                                },
                                Ok(false) => {
                                    cycles_of_no_work += 1;
                                    //TODO: based on timeout shutdown and report..
                                    if cycles_of_no_work > 10000 {
                                        cmd_mutex.lock().await.request_shutdown().await;
                                        error!("stopped on cycle of no work");// TODO: refine this.
                                    }
                                },
                                Err(e) => {
                                    cmd_mutex.lock().await.request_shutdown().await;
                                    error!("Internal Error: {:?}",e);
                                }
                            };
                        }
                    }
                }
            )
        })
    }
}

impl<C> IntoSimRunner<C> for SteadyStreamTx<StreamSimpleMessage>
where
    C: SteadyCommander + 'static,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        let this = self.clone();
        Box::new(move |cmd_mutex, responder, index| {
            Box::pin(
                {
                    let value = this.clone();
                    async move {
                        let mut that = value.lock().await;
                        let mut cycles_of_no_work = 0;
                        while cmd_mutex.lock().await.is_running(&mut || i!(that.mark_closed())) {
                            match responder.simulate_direction(&mut that, &cmd_mutex, index).await {
                                Ok(true) => {
                                    cycles_of_no_work = 0;
                                },
                                Ok(false) => {
                                    cycles_of_no_work += 1;
                                    //TODO: based on timeout shutdown and report..
                                    if cycles_of_no_work > 10000 {
                                        cmd_mutex.lock().await.request_shutdown().await;
                                        error!("stopped on cycle of no work");// TODO: refine this.
                                    }
                                },
                                Err(e) => {
                                    cmd_mutex.lock().await.request_shutdown().await;
                                    error!("Internal Error: {:?}",e);
                                }
                            };
                        }
                    }
                }
            )
        })
    }
}




/// Executes multiple simulation runners concurrently using the provided command context.
///
/// This function initializes an `Arc<Mutex<C>>` around the given `cmd` context and
/// drives each simulation runner in parallel, returning `Ok(())` when all simulations
/// complete successfully or an error if any simulation fails.
pub(crate) async fn simulated_behavior<C: SteadyCommander + 'static>(
    cmd: C,
    sims: Vec<&dyn IntoSimRunner<C>>,
) -> Result<(), Box<dyn Error>> {
    if let Some(responder) = cmd.sidechannel_responder() {
        let cmd_mutex = Arc::new(Mutex::new(cmd));
        let mut tasks: Vec<Pin<Box<dyn Future<Output = ()>>>> = Vec::new();
        // TODO: store the index position to match which we want along with type !!!
        for (i, behave) in sims.into_iter().enumerate() {
            let cmd_mutex = cmd_mutex.clone();
            let sim = behave.into_sim_runner();
            let task = sim(cmd_mutex, responder.clone(), i);
            tasks.push(task);
        }
        join_all(tasks).await;
    }
    Ok(())
}

#[cfg(test)]
mod simulate_edge_tests {
    use super::*;
    use std::sync::Arc;
    use futures_util::lock::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::future::Future;

    // Simple mock commander for testing
    #[derive(Clone)]
    struct TestCommander {
        running: Arc<AtomicBool>,
        call_count: Arc<AtomicUsize>,
        has_responder: bool,
    }

    impl TestCommander {
        fn new(running: bool, has_responder: bool) -> Self {
            Self {
                running: Arc::new(AtomicBool::new(running)),
                call_count: Arc::new(AtomicUsize::new(0)),
                has_responder,
            }
        }

        fn stop(&self) {
            self.running.store(false, Ordering::SeqCst);
        }

        fn call_count(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }


    #[test]
    fn test_sim_runner_type_creation() {
        let _ = crate::util::steady_logger::initialize();

        // Test that we can create the function type
        let _runner: SimRunner<TestCommander> = Box::new(|_cmd, _responder, _index| {
            Box::pin(async move {
                // Simple test runner that does nothing
            })
        });
    }

}
