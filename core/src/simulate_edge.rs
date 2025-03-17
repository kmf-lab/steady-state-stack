use std::error::Error;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use futures_util::future::join_all;
use futures_util::lock::Mutex;
use crate::{steady_rx, steady_tx, yield_now, SteadyCommander, SteadyTx, SteadyTxBundle};
use crate::graph_testing::SideChannelResponder;
use crate::SteadyRx;

pub type SimRunner<'a, C> = Box<
    dyn Fn(Arc<Mutex<C>>, SideChannelResponder) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + 'a
>;


pub trait IntoSimRunner<C: SteadyCommander + 'static> {
    fn into_sim_runner(&self) -> SimRunner<C>;
}


impl<T, C> IntoSimRunner<C> for EqualsBehavior<T>
where
    T: 'static + Debug + Send + Sync + Clone + Eq,
    C: SteadyCommander + 'static + Send,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        Box::new(move |cmd_mutex, responder| {
            Box::pin(
                self.clone().run_it(cmd_mutex, responder)
            )
        })
    }
}

impl<T, C> IntoSimRunner<C> for EchoBehavior<T>
where
    T: 'static + Debug + Send + Sync + Clone,
    C: SteadyCommander + 'static + Send,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        Box::new(move |cmd_mutex, responder| {
            Box::pin(
                self.clone().run_it(cmd_mutex, responder)
            )
        })
    }
}


// -----------------------------------------------------------------------------
// 2) EchoBehavior: does NOT require T: Eq
//    - simple tuple struct: EchoBehavior(tx)
// -----------------------------------------------------------------------------
#[derive(Clone)]
pub struct EchoBehavior<T>(pub SteadyTx<T>);
pub struct EchoBehaviorBundle<T, const GIRTH: usize>(pub SteadyTxBundle<T, GIRTH>);


impl<T> EchoBehavior<T>
where
    T: 'static + Debug + Send + Sync,
{
    async fn run_it<C: SteadyCommander>(
        self,
        cmd_mutex: Arc<Mutex<C>>,
        responder: SideChannelResponder,
    ) {
        let mut tx_guard = self.0.lock().await;
        while cmd_mutex.lock().await.is_running(&mut || tx_guard.mark_closed()) {
            responder
                .simulate_echo::<T, _, C>(&mut tx_guard, &cmd_mutex)
                .await;
            yield_now::yield_now().await;
        }
    }
}


// -----------------------------------------------------------------------------
// 3) EqualsBehavior: DOES require T: Eq
//    - simple tuple struct: EqualsBehavior(rx)
// -----------------------------------------------------------------------------
#[derive(Clone)]
pub struct EqualsBehavior<T>(pub SteadyRx<T>);

impl<T> EqualsBehavior<T>
where
    T: 'static + Debug + Send + Sync + Eq,
{
    async fn run_it<C: SteadyCommander>(
        self,
        cmd_mutex: Arc<Mutex<C>>,
        responder: SideChannelResponder,
    ) {
        let mut rx_guard = self.0.lock().await;
        while cmd_mutex.lock().await.is_running(&mut || rx_guard.is_closed_and_empty()) {
            responder
                .simulate_equals::<T, _, C>(&mut rx_guard, &cmd_mutex)
                .await;
            yield_now::yield_now().await;
        }
    }
}



pub(crate) async fn simulated_behavior< C: SteadyCommander + 'static, const LEN: usize,>(
    mut cmd: C, sims: [&dyn IntoSimRunner<C>; LEN],
) -> Result<(), Box<dyn Error>> {
    if let Some(responder) = cmd.sidechannel_responder() {
        let cmd_mutex = Arc::new(Mutex::new(cmd));
        let mut tasks: Vec<Pin<Box<dyn Future<Output = ()> + Send >>> = Vec::new();
        for behave in sims {
            let cmd_mutex = cmd_mutex.clone();
            let sim = behave.into_sim_runner();
            let task = sim(cmd_mutex, responder.clone());
            tasks.push(task);
        }
        join_all(tasks).await;
    }
    Ok(())
}