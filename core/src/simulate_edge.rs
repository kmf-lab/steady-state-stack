use std::error::Error;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use futures_util::future::join_all;
use futures_util::lock::Mutex;
use crate::{core_rx, yield_now, Rx, SteadyCommander, StreamRx, StreamRxBundleTrait, StreamSessionMessage, StreamSimpleMessage, StreamTx, Tx};
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::graph_testing::{SideChannelResponder, StageDirection};
use crate::i;

pub type SimRunner<C> = Box<
    dyn Fn(Arc<Mutex<C>>, SideChannelResponder, usize) -> Pin<Box<dyn Future<Output = ()>>>
>;

pub trait IntoSimRunner<C: SteadyCommander + 'static> {
    fn into_sim_runner(&self) -> SimRunner<C>;
}

impl<T, C> IntoSimRunner<C> for SimRx<T>
where
    T: 'static + Debug + Send + Sync + RxCore,
    <T as RxCore>::MsgOut: Debug + Eq + 'static,
    C: SteadyCommander + 'static,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        let this = self.clone();
        Box::new(move |cmd_mutex, responder, index| {
            Box::pin(
                <SimRx<T> as Clone>::clone(&this).run_it(cmd_mutex, responder, index)
            )
        })
    }
}

impl<T, C> IntoSimRunner<C> for SimTx<T>
where
    T: 'static + Debug + Send + Sync + TxCore,
    <T as TxCore>::MsgOut: Send + Sync + 'static,
    for<'a> <T as TxCore>::MsgIn<'a>: Debug,
    for<'a> <T as TxCore>::MsgIn<'a>: Clone,
    C: SteadyCommander + 'static,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        let this = self.clone();
        Box::new(move |cmd_mutex, responder, index| {
            Box::pin(
                <SimTx<T> as Clone>::clone(&this).run_it(cmd_mutex, responder, index)
            )
        })
    }
}

pub struct SimTx<T: TxCore>(pub Arc<Mutex<T>>);
impl<T: TxCore + 'static> SimTx<T> {
    async fn run_it<C: SteadyCommander>(self, cmd_mutex: Arc<Mutex<C>>, responder: SideChannelResponder, index: usize)
    where
        <T as TxCore>::MsgOut: Send + Sync,
        for<'a> <T as TxCore>::MsgIn<'a>: Debug,
        for<'a> <T as TxCore>::MsgIn<'a>: Clone,
    {
        let mut that = self.0.lock().await;
        while cmd_mutex.lock().await.is_running(&mut || that.shared_mark_closed()) {
            responder.simulate_direction(&mut that, &cmd_mutex, index).await;
            yield_now::yield_now().await;
        }
    }
}
impl<T: TxCore> Clone for SimTx<T> {
    fn clone(&self) -> Self {
        SimTx(self.0.clone())
    }
}

pub struct SimRx<T: RxCore>(pub Arc<Mutex<T>>);
impl<T: RxCore + 'static> SimRx<T> {
    async fn run_it<C: SteadyCommander>(self, cmd_mutex: Arc<Mutex<C>>, responder: SideChannelResponder, index: usize)
    where
        <T as RxCore>::MsgOut: Debug + Eq + 'static,
    {
        let mut that = self.0.lock().await;
        while cmd_mutex.lock().await.is_running(&mut || that.is_closed_and_empty()) {
            responder.simulate_wait_for(&mut that, &cmd_mutex, index).await;
            yield_now::yield_now().await;
        }
    }
}
impl<T: RxCore> Clone for SimRx<T> {
    fn clone(&self) -> Self {
        SimRx(self.0.clone())
    }
}

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
}