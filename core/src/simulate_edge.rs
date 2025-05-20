use std::error::Error;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use futures_util::future::join_all;
use futures_util::lock::{Mutex};
use crate::{yield_now, Rx, SteadyCommander, StreamRx, StreamSessionMessage, StreamSimpleMessage, StreamTx, Tx};
use crate::core_tx::TxCore;
use crate::graph_testing::SideChannelResponder;
use crate::i;

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
        while cmd_mutex.lock().await.is_running(&mut || i!(self.is_closed_and_empty())) {
            responder.simulate_equals(self, &cmd_mutex).await;
            yield_now::yield_now().await;
        }
    }
}
impl SimulateRx for StreamRx<StreamSimpleMessage> {
    async fn simulate_equals<C: SteadyCommander>(&mut self
                                                 , cmd_mutex: Arc<Mutex<C>>
                                                 , responder: SideChannelResponder) {
        while cmd_mutex.lock().await.is_running(&mut || i!(self.is_closed_and_empty())) {
            responder.simulate_equals(self, &cmd_mutex).await;
            yield_now::yield_now().await;
        }
    }
}
impl SimulateRx for StreamRx<StreamSessionMessage> {
    async fn simulate_equals<C: SteadyCommander>(&mut self
                                                 , cmd_mutex: Arc<Mutex<C>>
                                                 , responder: SideChannelResponder) {
        while cmd_mutex.lock().await.is_running(&mut || i!(self.is_closed_and_empty())) {
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


#[cfg(test)]
mod simulate_edge_tests {
    use super::*;
    use std::sync::Arc;
    use futures_util::lock::Mutex;

    // Test 1: Verify TestEcho can be instantiated and cloned
    #[test]
    fn test_echo_instantiation_and_clone() {
        // Create a simple struct implementing SimulateTx minimally
        struct DummyTx;
        impl SimulateTx for DummyTx {
            async fn simulate_echo<C: SteadyCommander>(
                &mut self,
                _cmd_mutex: Arc<Mutex<C>>,
                _responder: SideChannelResponder,
            ) {
                // No-op for testing
            }
        }

        let tx = Arc::new(Mutex::new(DummyTx));
        let echo = TestEcho(tx.clone());
        let echo_clone = echo.clone();

        // Check that cloning works by comparing Arc pointers
        assert!(Arc::ptr_eq(&echo.0, &echo_clone.0));
    }

    // Test 2: Verify TestEquals can be instantiated and cloned
    #[test]
    fn test_equals_instantiation_and_clone() {
        // Create a simple struct implementing SimulateRx minimally
        struct DummyRx;
        impl SimulateRx for DummyRx {
            async fn simulate_equals<C: SteadyCommander>(
                &mut self,
                _cmd_mutex: Arc<Mutex<C>>,
                _responder: SideChannelResponder,
            ) {
                // No-op for testing
            }
        }

        let rx = Arc::new(Mutex::new(DummyRx));
        let equals = TestEquals(rx.clone());
        let equals_clone = equals.clone();

        // Check that cloning works by comparing Arc pointers
        assert!(Arc::ptr_eq(&equals.0, &equals_clone.0));
    }



}