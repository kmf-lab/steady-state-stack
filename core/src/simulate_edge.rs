use std::error::Error;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use futures_util::future::join_all;
use futures_util::lock::Mutex;
use crate::{steady_rx, steady_tx, yield_now, SteadyCommander, SteadyTx};
use crate::graph_testing::SideChannelResponder;
use crate::SteadyRx;

#[derive(Clone)]
pub enum Behavior<T>  {
    Echo(SteadyTx<T>),
    Equals(SteadyRx<T>),
}

pub trait IntoSymRunner<C: SteadyCommander + 'static> {
    fn into_sym_runner(&self) -> SymRunner<C>;
}
impl<T, C> IntoSymRunner<C> for Behavior<T>
where
    T: 'static + Debug + Send + Sync + Eq + Clone,
    C: SteadyCommander + 'static + Send,
{
    fn into_sym_runner(&self) -> SymRunner<C> {

        Box::new(move |cmd_mutex, responder| {
            Box::pin(self.clone().run_it(cmd_mutex.clone(), responder.clone()))
        })
    }
}

pub type SymRunner<'a, C> = Box<
    dyn Fn(Arc<Mutex<C>>, SideChannelResponder) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + 'a
>;

impl<T: 'static + Debug + Send + Sync + Eq> Behavior<T> {

    async fn run_it<C: SteadyCommander>(
        self,
        cmd_mutex: Arc<Mutex<C>>,
        responder: SideChannelResponder,
    ) {
        
        match self {
            Behavior::Echo(tx) => {
                let mut tx = tx.lock().await;
                while cmd_mutex
                    .lock()
                    .await
                    .is_running(&mut || tx.mark_closed())
                {
                    responder
                        .simulate_echo::<
                            T,
                            futures::lock::MutexGuard<'_, steady_tx::Tx<T>>,
                            C,
                        >(&mut tx, &cmd_mutex.clone())
                        .await;
                    yield_now::yield_now().await;
                }
            }
            Behavior::Equals(rx) => {
                let mut rx = rx.lock().await;
                while cmd_mutex
                    .lock()
                    .await
                    .is_running(&mut || rx.is_closed_and_empty())
                {
                    responder
                        .simulate_equals::<
                            T,
                            futures::lock::MutexGuard<'_, steady_rx::Rx<T>>,
                            C,
                        >(&mut rx, &cmd_mutex.clone())
                        .await;
                    yield_now::yield_now().await;
                }
            }
        }
    }
}

pub(crate) async fn simulated_behavior< C: SteadyCommander + 'static, const LEN: usize,>(
    mut cmd: C,
    sims: [&dyn IntoSymRunner<C>; LEN],
) -> Result<(), Box<dyn Error>> {
    if let Some(responder) = cmd.sidechannel_responder() {
        let cmd_mutex = Arc::new(Mutex::new(cmd));
        let mut tasks: Vec<Pin<Box<dyn Future<Output = ()> + Send >>> = Vec::new();
        for behave in sims {
            let cmd_mutex = cmd_mutex.clone();
            let sim = behave.into_sym_runner();
            let task = sim(cmd_mutex, responder.clone());
            tasks.push(task);
        }
        join_all(tasks).await;
    }
    Ok(())
}