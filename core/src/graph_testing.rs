//! This module provides utilities for testing graphs in the SteadyState project.
//!
//! The module supports side channels for sending and receiving messages from actors in the graph.
//! This enables simulation of real-world scenarios and makes SteadyState stand out from other solutions
//! by providing a robust way to test graphs.

use std::any::Any;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;
use std::ops::{ DerefMut};
use std::sync::Arc;
use std::time::Duration;
use async_ringbuf::AsyncRb;
use async_ringbuf::consumer::AsyncConsumer;
use async_ringbuf::producer::AsyncProducer;
use log::*;
use futures_util::lock::{Mutex, MutexGuard};
use async_ringbuf::traits::Split;
use futures::channel::oneshot::Receiver;
use futures_util::future::FusedFuture;
use futures_util::select;
use ringbuf::consumer::Consumer;
use crate::{block_on, ActorIdentity, ActorName, Rx, RxBundle, SteadyCommander, Tx, TxBundle};
use crate::channel_builder::{ChannelBacking, InternalReceiver, InternalSender};
use ringbuf::traits::Observer;
use crate::abstract_executor_async_std::core_exec;
use crate::actor_builder::NodeTxRx;
use crate::commander::SendOutcome;
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use futures::FutureExt;
/// Represents the result of a graph test, which can either be `Ok` with a value of type `K`
/// or `Err` with a value of type `E`.
///
/// The `GraphTestResult` enum is used to encapsulate the outcome of testing a graph of actors
/// within the Steady State framework. It provides a way to represent either a successful test
/// result or an error encountered during the test.
#[derive(Debug)]
pub enum GraphTestResult<K, E>
    where
        K: Any + Send + Sync + Debug,
        E: Any + Send + Sync + Debug,
{
    /// Represents a successful test result.
    ///
    /// This variant contains the value `K`, which provides details about the successful outcome
    /// of the graph test.
    Ok(K),

    /// Represents a failed test result.
    ///
    /// This variant contains the value `E`, which provides details about the error encountered
    /// during the graph test.
    Err(E),
}




/// The `StageManager` struct manages side channels for nodes in the graph.
/// Each node holds its own lock on read and write to the backplane.
/// The backplane functions as a central message hub, ensuring that only one user can hold it at a time.
#[derive(Clone,Default)]
pub struct StageManager {
    node: HashMap<ActorName, Arc<NodeTxRx>>,
    pub(crate) backplane: HashMap<ActorName, Arc<Mutex<SideChannel>>>,
}

impl Debug for StageManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SideChannelHub")
            .field("node", &self.node)
            .finish()
    }
}

/// Type alias for a side channel, which is a pair of internal sender and receiver.
pub(crate) type SideChannel = (InternalSender<Box<dyn Any + Send + Sync>>, InternalReceiver<Box<dyn Any + Send + Sync>>);

trait StageAction {}

// Define StageDirection
pub enum StageDirection<T> {
    Echo(T),
    EchoAt(usize, T)

}

// Define StageWaitFor
pub enum StageWaitFor<T: Debug + Eq> { //TODO: if this used against a Tx will hang
    Message(T, Duration),
    MessageAt(usize, T, Duration),
    // AnyMessage
    // ValueGreaterThan
}

impl<T: Debug + Eq> StageAction for StageWaitFor<T> {}
impl<T: Debug + Clone> StageAction for StageDirection<T> {}


impl StageManager {


    /// Retrieves the transmitter and receiver for a node by its id.
    ///
    /// # Arguments
    ///
    /// * `key` - The name of the node.
    ///
    /// # Returns
    ///
    /// An `Option` containing an `Arc<Mutex<(SideChannel,Receiver<()>)>>` if the node exists.
    pub(crate) fn node_tx_rx(&self, key: ActorName) -> Option<Arc<NodeTxRx>> {
                self.node.get(&key).cloned()
    }

    /// Registers a new node with the specified name and capacity.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the node.
    /// * `capacity` - The capacity of the ring buffer.
    pub(crate) fn register_node(&mut self, key: ActorName, capacity: usize, shutdown_rx: Receiver<()>) -> bool {
            // Message to the node
            let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
            let (sender_tx, receiver_tx) = rb.split();

            // Response from the node
            let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
            let (sender_rx, receiver_rx) = rb.split();

            if self.node.contains_key(&key) || self.backplane.contains_key(&key)  {
                warn!("Node with name {:?} already exists, check suffix usage", key);     
                false
            } else {
                self.node.insert(key, Arc::new(Mutex::new(((sender_rx, receiver_tx), shutdown_rx) )));
                self.backplane.insert(key, Arc::new(Mutex::new((sender_tx, receiver_rx))));
                true
            }
    }



    pub fn actor_perform<S: StageAction + 'static + Send + Sync>(
        &self,
        name: &'static str,
        action: S,
    ) -> Result<Box<dyn Any + Send + Sync>, Box<dyn Error>> {
        self.call_actor_internal(Box::new(action), ActorName::new(name, None))
    }

    pub fn actor_perform_with_suffix<S: StageAction + 'static + Send + Sync>(
        &self,
        name: &'static str, suffix: usize,
        action: S,
    ) -> Result<Box<dyn Any + Send + Sync>, Box<dyn Error>> {
        self.call_actor_internal(Box::new(action), ActorName::new(name, Some(suffix)))
    }


        /// Sends a message to a node and waits for a response.
    ///
    /// # Arguments
    ///
    /// * `msg` - The message to send.
    /// * `name` - The name of the node.
    ///
    /// # Returns
    ///
    /// An `Option` containing the response message if the operation is successful.
    ///
    pub(crate) fn call_actor_internal(&self, msg: Box<dyn Any + Send + Sync>, id: ActorName) -> Result<Box<dyn Any + Send + Sync>, Box<dyn Error>> {

        if let Some(sc) = self.backplane.get(&id) {            
            core_exec::block_on( async move {
                let mut sc_guard = sc.lock().await;
                let (tx, rx) = sc_guard.deref_mut();
                match tx.push(msg).await {
                    Ok(_) => {
                        if let Some(response) = rx.pop().await {
                            let is_ok = response.downcast_ref::<&str>()
                                .map(|msg| *msg == OK_MESSAGE)
                                .or_else(|| response.downcast_ref::<String>()
                                    .map(|msg| msg == OK_MESSAGE))
                                .unwrap_or(false);
                            if is_ok {
                                Ok(response)
                            } else {
                                Err("Actor responded with unexpected message".into())
                            }
                        } else {
                            error!("Actor responded unexpected message");
                            Err("Actor disconnected, no response".into())
                        }                        
                    }
                    Err(e) => {
                        error!("Error sending test request: {:?}", e);
                        Err("Unable to send request, see logs".into())
                    }
                }
            })            
        } else {
            error!("Actor with name {:?} not found", id);
            Err("Unable to find the target actor.".into())
        }
    }
}

/// The `SideChannelResponder` struct provides a way to respond to messages from a side channel.
#[derive(Clone)]
pub struct SideChannelResponder {
    pub(crate) arc: Arc<Mutex<(SideChannel,Receiver<()>)>>,
    pub(crate) identity: ActorIdentity,
}

const WAIT_FOR_QUANT:Duration = Duration::from_millis(50);
pub (crate) const OK_MESSAGE: &'static str = "ok";

impl SideChannelResponder {

    //   Ok(true)  //did something keep going
    //   Ok(false) //did nothing becuase it does not pertain - if we are here beyond timeout we must trigger shutdown
    //   Err("")  // exit now failure, shutdown.
    pub async fn simulate_direction<'a, T: 'static + Debug + Clone, X: TxCore<MsgIn<'a> = T>, C: SteadyCommander>(&self
                                                                                                                  , tx_core: &mut X, cmd: & Arc<Mutex<C>>
                                                                                                                  , index: usize) -> Result<bool, Box<dyn Error>>
    where <X as TxCore>::MsgOut: Send, <X as TxCore>::MsgOut: Sync, <X as TxCore>::MsgOut: 'static {

            // trace!("should echo does apply, waiting for vacant unit");
            if tx_core.shared_wait_vacant_units(tx_core.one()).await {
                //NOTE: we block here await until some direction comes in.
                self.respond_with(move |message,cmd_guard| {
                   match message.downcast_ref::<StageDirection<X::MsgIn<'a>>>() {
                        Some(msg) => {
                            match  msg {
                                StageDirection::Echo(m) => {
                                    match cmd_guard.try_send(tx_core,m.clone()) {
                                        SendOutcome::Success => {Some(Box::new(OK_MESSAGE))}
                                        SendOutcome::Blocked(msg) => {Some(Box::new(msg))}
                                    }
                                }
                                StageDirection::EchoAt(i, m) => {
                                    //trace!("echo at {} vs {} ", i, index);
                                    if *i == index {
                                        match cmd_guard.try_send(tx_core, m.clone()) {
                                            SendOutcome::Success => { Some(Box::new(OK_MESSAGE)) }
                                            SendOutcome::Blocked(msg) => { Some(Box::new(msg)) }
                                        }
                                    } else {
                                        None //this is not for us because we want index i
                                    }
                                }
                            }
                        },
                        None => {
                            error!("Unable to cast stage direction to target type: {}", std::any::type_name::<StageDirection<X::MsgIn<'a>>>());
                            None
                        }
                    }


                },cmd).await
            } else {
                Ok(true)
            }


    }

    //   Ok(true)  //did something keep going
    //   Ok(false) //did nothing becuase it does not pertain - if we are here beyond timeout we must trigger shutdown
    //   Err("")  // exit now failure, shutdown.
    pub async fn simulate_wait_for<T: Debug + Eq + 'static, X: RxCore<MsgOut = T>, C: SteadyCommander>(&self, rx_core: &mut X
                                                                                                           , cmd: & Arc<Mutex<C>>
                                                                                                           , index: usize) -> Result<bool, Box<dyn Error>>
    where <X as RxCore>::MsgOut: std::fmt::Debug {
        //TODO: can we add index check here as well?

        match self.should_apply::<StageWaitFor<T>>().await {
            Some(true) => {} //ok continue with our task
            Some(false) => return Ok(false), //do not match
            None => return Ok(true) //nothing found
        }

            if rx_core.shared_wait_avail_units(1).await {
                    self.respond_with(move |message,cmd_guard| {
                    let wait_for: &StageWaitFor<T> = message.downcast_ref::<StageWaitFor<T>>()
                                                        .expect("error casting");
                    //TODO: must return not appicable

                    let message = match wait_for {
                        StageWaitFor::Message(m,t) => Some((m,t)),
                        StageWaitFor::MessageAt(i,m,t) => if *i == index {Some((m,t))}
                                                                                   else {None}
                    };
                    use std::time::{Instant};

                    if let Some((expected, timeout)) = message {
                        let start = Instant::now();
                        loop {
                            match cmd_guard.try_take(rx_core) {
                                Some(measured) => {
                                    if expected.eq(&measured) {
                                        break Some(Box::new(OK_MESSAGE));
                                    } else {
                                        //error!("not equals {:?} vs {:?}",m,&measured);
                                        let failure = format!("no match {:?} {:?}", expected, measured);
                                        break Some(Box::new(failure)); //TODO: response string is not getting checked!!!!
                                    }
                                }
                                None => {
                                    //error!("no task value found");
                                    if start.elapsed() >= *timeout {
                                        break Some(Box::new("timeout, no message".to_string()));
                                    } else {
                                        //we are in a lambda so we can not use await
                                        //this is for testing so this is not so bad
                                        std::thread::sleep(WAIT_FOR_QUANT);
                                    }
                                }
                            }
                        }
                    } else {
                        None //not applicable by index
                    }

                },cmd).await
            } else {
                Ok(true)
            }

    }


    /// Creates a new `SideChannelResponder`.
    ///
    /// # Arguments
    ///
    /// * `arc` - An `Arc<Mutex<SideChannel>>` for the side channel.
    ///
    /// # Returns
    ///
    /// A new `SideChannelResponder` instance.
    pub fn new(arc: Arc<Mutex<(SideChannel,Receiver<()>)>>, identity: ActorIdentity) -> Self {
        SideChannelResponder { arc, identity}
    }

    /// Wait for requests which will need a response
    ///
    /// # Arguments
    ///
    /// * `count` - how many requests to wait for, must be less than length of the channel
    ///
    /// # Returns
    ///
    /// True if the count is met and False if we must return because a shutdown is in process
    pub async fn wait_available_units(&mut self, count: usize) -> bool {
        let mut guard = self.arc.lock().await;
        let ((_tx, rx), shutdown) = guard.deref_mut();

        if rx.occupied_len() >= count {
            true
        } else {
            let mut one_down = shutdown;
            if !one_down.is_terminated() {
                let mut operation = &mut rx.wait_occupied(count);
                select! { _ = one_down => false, _ = operation => true }
            } else {
                false
            }
        }
    }


    /// An async function that listens for a message and echoes it to all outgoing channels in a bundle.
    ///
    /// # Parameters
    /// - `cmd`: A mutable reference to the SteadyCommander handling command operations.
    /// - `target_tx_bundle`: A mutable reference to the bundle of outgoing channels to send the echoed message.
    ///
    /// # Returns
    /// - `bool`: `true` if the operation succeeded; otherwise, `false`.
    pub async fn echo_responder_bundle<M: 'static + Clone + Debug + Send, C: SteadyCommander>(
        &self,
        cmd: &mut Arc<Mutex<C>>,
        target_tx_bundle: &mut TxBundle<'_,M>,
    ) -> Result<bool,Box<dyn Error>> {
        if let Some(true) = self.should_apply::<M>().await {
            let girth = target_tx_bundle.len();

            let mut cmd_guard = cmd.lock().await;
            for t in target_tx_bundle.iter_mut() {
                if !cmd_guard.wait_vacant(&mut *t, 1).await {
                    return Ok(true);
                };
            }

            self.respond_with(|message, cmd| {
                let msg = message.downcast_ref::<M>().expect("error casting");
                let total: usize = (0..girth)
                    .filter(|&c| {
                        cmd.try_send(&mut target_tx_bundle[c], msg.clone()).is_sent()
                    })
                    .count();
    
                if total == girth {
                    Some(Box::new("ok".to_string()))
                } else {
                    let failure = format!("failed to echo to {:?} channels", girth - total);
                    Some(Box::new(failure))
                }
            },cmd).await
        } 
        else 
        { Ok(false) }
    }


    /// An async function that verifies a message matches all incoming messages from a bundle of channels.
    ///
    /// # Parameters
    /// - `cmd`: A mutable reference to the SteadyCommander handling command operations.
    /// - `source_rx`: A mutable reference to the bundle of incoming channels to read messages from.
    ///
    /// # Returns
    /// - `bool`: `true` if the operation succeeded; otherwise, `false`.
    pub async fn equals_responder_bundle<M: 'static + Clone + Debug + Send + Eq, C: SteadyCommander>(
        &self,
        cmd: &mut Arc<Mutex<C>>,
        source_rx: &mut RxBundle<'_, M>,
    ) -> Result<bool, Box<dyn Error>> {
        if let Some(true) = self.should_apply::<M>().await {
            let girth = source_rx.len();

            let mut cmd_guard = cmd.lock().await;
            for x in 0..girth {
                let srx: &mut MutexGuard<Rx<M>> =  &mut source_rx[x];
                if !cmd_guard.wait_avail(srx, 1).await {
                    return Ok(true);
                };
            }

            self.respond_with(|message,cmd| {
                let msg: &M = message.downcast_ref::<M>().expect("error casting");
                let total = (0..girth)
                    .filter_map(|c| cmd.try_take(&mut source_rx[c]))
                    .filter(|m| m.eq(msg))
                    .count();

                if girth == total {
                    Some(Box::new("ok".to_string()))
                } else {
                    let failure = format!("match failure {:?} of {:?}", msg, girth - total);
                    Some(Box::new(failure))
                }
            },cmd).await
        } else {
            Ok(false)
        }
    }

    /// Checks if the next message in the queue matches the expected type without consuming it.
    ///
    /// # Type Parameters
    /// - `M`: The expected message type.
    pub async fn should_apply<M: 'static>(&self) -> Option<bool>
    {
        let mut guard = self.arc.lock().await;
        let ((_, rx), _) = guard.deref_mut();

        // Attempt to peek at the next message
        if let Some(q) = rx.try_peek() {
            let is_correct_type = q.is::<M>();
            if !is_correct_type {
                error!("Fix the unit test sent message, Type must match channel of the target actor but it does not.");
            }
            // Check if the message can be downcast to the expected type
            Some(is_correct_type)
        } else {
            // No message available to peek at
            None
        }
    }
    
    /// Responds to messages from the side channel using the provided function.
    ///
    /// # Arguments
    ///
    /// * `f` - A function that takes a message and returns a response.
    ///
    ///
    pub async fn respond_with<F,C>(&self, mut f: F, cmd: & Arc<Mutex<C>>) -> Result<bool, Box<dyn Error>>
        where
            C: SteadyCommander,
            F: FnMut(&Box<dyn Any + Send + Sync>, &mut C) -> Option<Box<dyn Any + Send + Sync>>,
    {
        let mut guard = self.arc.lock().await;
        let ((tx, rx), shutdown) = guard.deref_mut();

        //important to allow other simulators we must wait until we have at lest one thing to do


        // Wait for either shutdown or for the receiver to have at least one message
        select! {
            _ = shutdown.fuse() => {
                //trace!("shutdown detected in simulator");
                // Shutdown signal detected
            },
            _ = rx.wait_occupied(1) => {
                // Message available, proceed to process
                //trace!("we have respond with commands {}", rx.occupied_len());
            }
        }

        //that thing might not be for us so we peek it first
        if let Some(q) = rx.try_peek() {
            //NOTE: if a shutdown is called for and we are not able to push this message
            //      it will result in a timeout and error by design for testing. This will
            //      happen if the main test block is not consuming the answers to the questions
            //NOTE: this should never block on shutdown since above we immediately pull the answer

            let mut cmd_guard = cmd.lock().await;

            if let Some(r) = f(q, &mut cmd_guard) {
                match tx.push(r).await {
                    Ok(_) => {
                        let _ = rx.try_pop(); //we know f(q) accepted it so now remove it from the channel.
                        Ok(true)
                    }
                    Err(e) => {
                        error!("Error sending test implementation response: {:?} Identity: {:?}", e,self.identity);
                        Err("internal error pushing response".into()) //we have an internal fault to report
                    }
                }
            } else {
                Ok(false) // function did not consume task for us, not applicable
            }
        } else {
            Ok(true)
        }
    }
}


#[cfg(test)]
mod graph_testing_tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::OnceLock;
    use parking_lot::RwLock;
    use std::time::Instant;
    use super::*;
    use futures::channel::oneshot;
    use crate::{GraphLiveliness, LazySteadyRx, LazySteadyTx, Rx, SteadyCommander};
    use crate::channel_builder::ChannelBuilder;
    use crate::commander_context::SteadyContext;
    use crate::monitor::ActorMetaData;
    use crate::core_tx::TxCore;

    #[test]
    fn test_graph_test_result_ok() {
        let result: GraphTestResult<i32, &str> = GraphTestResult::Ok(42);
        if let GraphTestResult::Ok(value) = result {
            assert_eq!(value, 42);
        } else {
            panic!("Expected Ok variant");
        }
    }

    #[test]
    fn test_graph_test_result_err() {
        let result: GraphTestResult<i32, &str> = GraphTestResult::Err("error");
        if let GraphTestResult::Err(value) = result {
            assert_eq!(value, "error");
        } else {
            panic!("Expected Err variant");
        }
    }


    #[test]
    fn test_register_and_retrieve_node() {
        let mut hub = StageManager::default();

        let actor = ActorIdentity::new(2,"test_actor",Some(2));

        let (_tx,rx) = oneshot::channel();

        hub.register_node(actor.label, 10, rx );

        let node = hub.node_tx_rx(actor.label);
        assert!(node.is_some(), "Node should be registered and retrievable");
    }





    // Helper method to build tx and rx arguments
    fn build_tx_rx() -> (oneshot::Sender<()>, oneshot::Receiver<()>) {
        oneshot::channel()
    }

    // Common function to create a test SteadyContext
    fn test_steady_context() -> SteadyContext {
        let (_tx, rx) = build_tx_rx();
        SteadyContext {
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
                Default::default(),
                Default::default()
            ))),
            channel_count: Arc::new(AtomicUsize::new(0)),
            ident: ActorIdentity::new(0, "test_actor", None),
            args: Arc::new(Box::new(())),
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())),
            actor_metadata: Arc::new(ActorMetaData::default()),
            oneshot_shutdown_vec: Arc::new(Mutex::new(Vec::new())),
            oneshot_shutdown: Arc::new(Mutex::new(rx)),
            node_tx_rx: None,
            instance_id: 0,
            last_periodic_wait: Default::default(),
            is_in_graph: true,
            actor_start_time: Instant::now(),
            frame_rate_ms: 1000,
            team_id: 0,
            show_thread_info: false,
            aeron_meda_driver: OnceLock::new(),
            use_internal_behavior: true,
            shutdown_barrier: None,
        }
    }

    // Helper function to create a new Rx instance
    fn create_rx<T: std::fmt::Debug>(data: Vec<T>) -> Arc<Mutex<Rx<T>>> {
        let (tx, rx) = create_test_channel();

        let send = tx.clone();
        if let Some(ref mut send_guard) = send.try_lock() {
            for item in data {
                let _ = send_guard.shared_try_send(item);
            }
        }
        rx.clone()
    }

    fn create_test_channel<T: std::fmt::Debug>() -> (LazySteadyTx<T>, LazySteadyRx<T>) {
        let builder = ChannelBuilder::new(
            Arc::new(Default::default()),
            Arc::new(Default::default()),
            40);

        builder.build_channel::<T>()
    }

    // Test for wait_avail_units method
    #[async_std::test]
    async fn test_wait_avail_units() {
        let context = test_steady_context();
        let rx = create_rx(vec![1, 2, 3]);
        if let Some(mut rx) = rx.try_lock() {
            let result = context.wait_avail(&mut rx, 3).await;
            assert!(result);
        };
    }

}



