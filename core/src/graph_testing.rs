//! This module provides utilities for testing graphs in the SteadyState project.
//!
//! The module supports side channels for sending and receiving messages from actors in the graph.
//! This enables simulation of real-world scenarios and makes SteadyState stand out from other solutions
//! by providing a robust way to test graphs.

use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::DerefMut;
use std::sync::Arc;
use async_ringbuf::AsyncRb;
use async_ringbuf::consumer::AsyncConsumer;
use async_ringbuf::producer::AsyncProducer;
use log::error;
use futures_util::lock::Mutex;
use async_ringbuf::traits::Split;
use futures::channel::oneshot::Receiver;
use futures_util::future::FusedFuture;
use futures_util::select;
use ringbuf::consumer::Consumer;
use crate::{ActorIdentity, ActorName};
use crate::channel_builder::{ChannelBacking, InternalReceiver, InternalSender};

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


/// Type alias for a side channel, which is a pair of internal sender and receiver.
pub(crate) type SideChannel = (InternalSender<Box<dyn Any + Send + Sync>>, InternalReceiver<Box<dyn Any + Send + Sync>>);



/// The `SideChannelHub` struct manages side channels for nodes in the graph.
/// Each node holds its own lock on read and write to the backplane.
/// The backplane functions as a central message hub, ensuring that only one user can hold it at a time.
#[derive(Default)]
pub struct SideChannelHub {
    node: HashMap<ActorName, Arc<Mutex<SideChannel>>>,
    pub(crate) backplane: HashMap<ActorName, SideChannel>,
}

impl Debug for SideChannelHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SideChannelHub")
            .field("node", &self.node)
            .finish()
    }
}

impl SideChannelHub {


    /// Retrieves the transmitter and receiver for a node by its id.
    ///
    /// # Arguments
    ///
    /// * `key` - The name of the node.
    ///
    /// # Returns
    ///
    /// An `Option` containing an `Arc<Mutex<SideChannel>>` if the node exists.
    pub fn node_tx_rx(&self, key: ActorName) -> Option<Arc<Mutex<SideChannel>>> {
                self.node.get(&key).cloned()
    }

    /// Registers a new node with the specified name and capacity.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the node.
    /// * `capacity` - The capacity of the ring buffer.
    pub fn register_node(&mut self, key: ActorName, capacity: usize) {
            // Message to the node
            let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
            let (sender_tx, receiver_tx) = rb.split();

            // Response from the node
            let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
            let (sender_rx, receiver_rx) = rb.split();

            if self.node.get(&key).is_some() {
                error!("Node with name {:?} already exists, check suffix usage", key);
            }
            if self.backplane.get(&key).is_some() {
                error!("Node with name {:?} already exists, check suffix usage", key);
            }

            self.node.insert(key, Arc::new(Mutex::new((sender_rx, receiver_tx))));
            self.backplane.insert(key, (sender_tx, receiver_rx));

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
    pub async fn node_call(&mut self, msg: Box<dyn Any + Send + Sync>, id: ActorName) -> Option<Box<dyn Any + Send + Sync>> {

        if let Some((tx, rx)) = self.backplane.get_mut(&id) {
            match tx.push(msg).await {
                Ok(_) => {
                    //do nothing else but immediately get the response for more deterministic testing
                    return rx.pop().await;
                }
                Err(e) => {
                    error!("Error sending test implementation request: {:?}", e);
                }
            }
        }
        None
    }
}

/// The `SideChannelResponder` struct provides a way to respond to messages from a side channel.
pub struct SideChannelResponder {
    pub(crate) arc: Arc<Mutex<SideChannel>>,
    pub(crate) shutdown: Arc<Mutex<Receiver<()>>>,
}

impl SideChannelResponder {
    /// Creates a new `SideChannelResponder`.
    ///
    /// # Arguments
    ///
    /// * `arc` - An `Arc<Mutex<SideChannel>>` for the side channel.
    ///
    /// # Returns
    ///
    /// A new `SideChannelResponder` instance.
    pub fn new(arc: Arc<Mutex<SideChannel>>, shutdown: Arc<Mutex<Receiver<()>>>) -> Self {
        SideChannelResponder { arc, shutdown}
    }

    /// Responds to messages from the side channel using the provided function.
    ///
    /// # Arguments
    ///
    /// * `f` - A function that takes a message and returns a response.
    ///
    pub async fn respond_with<F>(&self, mut f: F)
        where
            F: FnMut(Box<dyn Any + Send + Sync>) -> Box<dyn Any + Send + Sync>,
    {
        let mut guard = self.arc.lock().await;
        let (ref mut tx, ref mut rx) = guard.deref_mut();


        //wait for message unless we see the shutdown signal
        let question: Option<Box<dyn Any + Send + Sync>> = {
            let mut shutdown_guard = self.shutdown.lock().await;
            let mut one_down = &mut shutdown_guard.deref_mut();
            if !one_down.is_terminated() {
                let mut operation = &mut rx.pop();
                select! { _ = one_down => rx.try_pop(),
                     p = operation => p }
            } else {
                rx.try_pop()
            }
        };

        if let Some(q) = question {

            //NOTE: if a shutdown is called for and we are not able to push this message
            //      it will result in a timeout and error by design for testing. This will
            //      happen if the main test block is not consuming the answers to the questions
            //NOTE: this should never block on shutdown since above we immediately pull the answer
            match tx.push(f(q)).await {
                Ok(_) => {}
                Err(e) => {
                    error!("Error sending test implementation response: {:?}", e);
                }
            }
        }
    }



}


#[cfg(test)]
mod graph_testing_tests {
    use super::*;
    use async_std::test;
    use futures::channel::oneshot;

    #[test]
    async fn test_graph_test_result_ok() {
        let result: GraphTestResult<i32, &str> = GraphTestResult::Ok(42);
        if let GraphTestResult::Ok(value) = result {
            assert_eq!(value, 42);
        } else {
            panic!("Expected Ok variant");
        }
    }

    #[test]
    async fn test_graph_test_result_err() {
        let result: GraphTestResult<i32, &str> = GraphTestResult::Err("error");
        if let GraphTestResult::Err(value) = result {
            assert_eq!(value, "error");
        } else {
            panic!("Expected Err variant");
        }
    }


    #[test]
    async fn test_register_and_retrieve_node() {
        let mut hub = SideChannelHub::default();
        let actor_name = ActorName::new("test_actor",None);

        hub.register_node(actor_name.clone(), 10);

        let node = hub.node_tx_rx(actor_name.clone());
        assert!(node.is_some(), "Node should be registered and retrievable");
    }

    // #[test]
    // async fn test_node_call_success() {
    //     let mut hub = SideChannelHub::default();
    //     let actor_name = ActorName::new("test_actor",None);
    //
    //     hub.register_node(actor_name.clone(), 10);
    //
    //     let response_msg = Box::new(42) as Box<dyn Any + Send + Sync>;
    //     let msg = Box::new(21) as Box<dyn Any + Send + Sync>;
    //
    //     {
    //         // Manually simulate the receiving process
    //         let temp = hub.backplane.get_mut(&actor_name);
    //         let (sender, receiver) = temp.unwrap();
    //
    //         let _ = sender.push(response_msg).await;
    //
    //     }
    //
    //     let result = hub.node_call(msg, actor_name).await;
    //     assert!(result.is_some(), "Should receive a response");
    //     let r = result.unwrap();
    //     let response = r.downcast_ref::<i32>().unwrap();
    //     assert_eq!(*response, 42, "Response should match the expected value");
    // }

    #[test]
    async fn test_node_call_error() {
        let mut hub = SideChannelHub::default();
        let actor_name = ActorName::new("test_actor",None);

        hub.register_node(actor_name.clone(), 10);

        let msg = Box::new(21) as Box<dyn Any + Send + Sync>;

        // Drop the backplane's sender to simulate an error scenario
        hub.backplane.remove(&actor_name);

        let result = hub.node_call(msg, actor_name).await;
        assert!(result.is_none(), "Should return None if the sender is not available");
    }

    // #[test]
    // async fn test_side_channel_responder() {
    //     let mut hub = SideChannelHub::default();
    //     let actor_name = ActorName::new("test_actor",None);
    //
    //     hub.register_node(actor_name.clone(), 10);
    //
    //     let node = hub.node_tx_rx(actor_name.clone()).unwrap();
    //     let (shutdown_tx, shutdown_rx) = oneshot::channel();
    //
    //     let responder = SideChannelResponder::new(node.clone(), Arc::new(Mutex::new(shutdown_rx)));
    //
    //     async_std::task::spawn(async move {
    //         responder.respond_with(|msg| {
    //             let received_msg = msg.downcast_ref::<i32>().unwrap();
    //             Box::new(received_msg * 2) as Box<dyn Any + Send + Sync>
    //         }).await;
    //     });
    //
    //     let msg = Box::new(21) as Box<dyn Any + Send + Sync>;
    //     let result = hub.node_call(msg, actor_name).await;
    //
    //     assert!(result.is_some(), "Should receive a response from the responder");
    //     let response = result.unwrap().downcast_ref::<i32>().unwrap();
    //     assert_eq!(*response, 42, "Response should be the doubled value of the message sent");
    // }

    // #[test]
    // async fn test_side_channel_responder_shutdown() {
    //     let mut hub = SideChannelHub::default();
    //     let actor_name = ActorName::new("test_actor",None);
    //
    //     hub.register_node(actor_name.clone(), 10);
    //
    //     let node = hub.node_tx_rx(actor_name.clone()).unwrap();
    //     let (shutdown_tx, shutdown_rx) = oneshot::channel();
    //
    //     let responder = SideChannelResponder::new(node.clone(), Arc::new(Mutex::new(shutdown_rx)));
    //
    //     async_std::task::spawn(async move {
    //         responder.respond_with(|msg| {
    //             let received_msg = msg.downcast_ref::<i32>().unwrap();
    //             Box::new(received_msg * 2) as Box<dyn Any + Send + Sync>
    //         }).await;
    //     });
    //
    //     // Send a shutdown signal
    //     shutdown_tx.send(()).unwrap();
    //
    //     let msg = Box::new(21) as Box<dyn Any + Send + Sync>;
    //     let result = hub.node_call(msg, actor_name).await;
    //
    //     assert!(result.is_none(), "Should not receive a response after shutdown");
    // }

}
