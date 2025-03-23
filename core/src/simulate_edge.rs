use std::error::Error;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use futures_util::future::join_all;
use futures_util::lock::{Mutex};
use crate::{yield_now, Rx, SteadyCommander, StreamRx, StreamSessionMessage, StreamSimpleMessage, StreamTx, Tx};
use crate::core_tx::TxCore;
use crate::graph_testing::SideChannelResponder;


pub type SimRunner<C> = Box<
    dyn Fn(Arc<Mutex<C>>, SideChannelResponder) -> Pin<Box<dyn Future<Output = ()> >>
>;


pub trait IntoSimRunner<C: SteadyCommander + 'static> {
    fn into_sim_runner(&self) -> SimRunner<C>;
}


impl<T, C> IntoSimRunner<C> for TestEquals<T>
where
    T: 'static + Debug + Send + Sync + SimulateRx,
    C: SteadyCommander + 'static,
{
    fn into_sim_runner(&self) -> SimRunner<C> {

         let this= <TestEquals<T> as Clone>::clone(self);
         Box::new( move |cmd_mutex, responder| {
             Box::pin(
                 <TestEquals<T> as Clone>::clone(&this).run_it(cmd_mutex, responder)
             )
         })
    }
}

impl<T, C> IntoSimRunner<C> for TestEcho<T>
where
    T: 'static + Debug + Send + Sync + SimulateTx,
    C: SteadyCommander + 'static ,
{
    fn into_sim_runner(&self) -> SimRunner<C> {
        let this = <TestEcho<T> as Clone>::clone(&self);
        Box::new(move |cmd_mutex, responder| {
            Box::pin(
                <TestEcho<T> as Clone>::clone(&this).run_it(cmd_mutex, responder)
            )
        })
    }
}

pub trait SimulateTx {

    #[allow(async_fn_in_trait)]
    async fn simulate_echo<C: SteadyCommander>( &mut self
                                         , cmd_mutex: Arc<Mutex<C>>
                                         , responder: SideChannelResponder) ;
}
pub trait SimulateRx {

    #[allow(async_fn_in_trait)]
    async fn simulate_equals<C: SteadyCommander>( &mut self
                                                , cmd_mutex: Arc<Mutex<C>>
                                                , responder: SideChannelResponder) ;
}


impl<T: Send + Sync + Clone + 'static> SimulateTx for Tx<T> {
    async fn simulate_echo<C: SteadyCommander>(&mut self
                         , cmd_mutex: Arc<Mutex<C>>
                         , responder: SideChannelResponder) {
         while cmd_mutex.lock().await.is_running(&mut || self.shared_mark_closed()) {
             responder.simulate_echo(self, &cmd_mutex).await;
             yield_now::yield_now().await;
         }
    }
}
impl SimulateTx for StreamTx<StreamSimpleMessage> {
    async fn simulate_echo<C: SteadyCommander>(&mut self
                                               , cmd_mutex: Arc<Mutex<C>>
                                               , responder: SideChannelResponder) {
        while cmd_mutex.lock().await.is_running(&mut || self.shared_mark_closed()) {
            responder.simulate_echo(self, &cmd_mutex).await;
            yield_now::yield_now().await;
        }
    }
}
impl SimulateTx for StreamTx<StreamSessionMessage> {
    async fn simulate_echo<C: SteadyCommander>(&mut self
                                               , cmd_mutex: Arc<Mutex<C>>
                                               , responder: SideChannelResponder) {
        while cmd_mutex.lock().await.is_running(&mut || self.shared_mark_closed()) {
            responder.simulate_echo(self, &cmd_mutex).await;
            yield_now::yield_now().await;
        }
    }
}



impl<T: Send + Sync + Debug + Clone + Eq + 'static> SimulateRx for Rx<T> {
    async fn simulate_equals<C: SteadyCommander>(&mut self
                                               , cmd_mutex: Arc<Mutex<C>>
                                               , responder: SideChannelResponder) {
        while cmd_mutex.lock().await.is_running(&mut || self.is_closed_and_empty()) {
            responder.simulate_equals(self, &cmd_mutex).await;
            yield_now::yield_now().await;
        }
    }
}
impl SimulateRx for StreamRx<StreamSimpleMessage> {
    async fn simulate_equals<C: SteadyCommander>(&mut self
                                                 , cmd_mutex: Arc<Mutex<C>>
                                                 , responder: SideChannelResponder) {
        while cmd_mutex.lock().await.is_running(&mut || self.is_closed_and_empty()) {
            responder.simulate_equals(self, &cmd_mutex).await;
            yield_now::yield_now().await;
        }
    }
}
impl SimulateRx for StreamRx<StreamSessionMessage> {
    async fn simulate_equals<C: SteadyCommander>(&mut self
                                                 , cmd_mutex: Arc<Mutex<C>>
                                                 , responder: SideChannelResponder) {
        while cmd_mutex.lock().await.is_running(&mut || self.is_closed_and_empty()) {
            responder.simulate_equals(self, &cmd_mutex).await;
            yield_now::yield_now().await;
        }
    }
}
// -----------------------------------------------------------------------------

pub struct TestEcho<T: SimulateTx >(pub Arc<Mutex<T>>);
impl<T: SimulateTx> TestEcho<T>
{
    async fn run_it<C: SteadyCommander>( self, cmd_mutex: Arc<Mutex<C>>, responder: SideChannelResponder,
    ) {
        self.0.lock().await.simulate_echo(cmd_mutex,responder).await;
    }
}
impl<T: SimulateTx> Clone for TestEcho<T> {
    fn clone(&self) -> Self {
        TestEcho(self.0.clone())
    }
}

pub struct TestEquals<T: SimulateRx >(pub Arc<Mutex<T>>);
impl<T: SimulateRx> TestEquals<T>
{
    async fn run_it<C: SteadyCommander>( self, cmd_mutex: Arc<Mutex<C>>, responder: SideChannelResponder,
    ) {
        self.0.lock().await.simulate_equals(cmd_mutex,responder).await;
    }
}
impl<T: SimulateRx> Clone for TestEquals<T> {
    fn clone(&self) -> Self {
        TestEquals(self.0.clone())
    }
}



pub(crate) async fn simulated_behavior< C: SteadyCommander + 'static>(
    cmd: C, sims: Vec<&dyn IntoSimRunner<C>>,
) -> Result<(), Box<dyn Error>> {
    if let Some(responder) = cmd.sidechannel_responder() {
        let cmd_mutex = Arc::new(Mutex::new(cmd));
        let mut tasks: Vec<Pin<Box<dyn Future<Output = ()>  >>> = Vec::new();
        for behave in sims.into_iter() {
            let cmd_mutex = cmd_mutex.clone();
            let sim = behave.into_sim_runner();
            let task = sim(cmd_mutex, responder.clone());
            tasks.push(task);
        }
        join_all(tasks).await;
    }
    Ok(())
}



struct SimRunnerCollection<T: 'static> {
    storage: Vec<Box<dyn IntoSimRunner<T>>>, // Owns different test types
    references: Vec<&'static mut dyn IntoSimRunner<T>>, // References for usage
}

