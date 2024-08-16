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
use std::time::Duration;
use async_ringbuf::AsyncRb;
use async_ringbuf::consumer::AsyncConsumer;
use async_ringbuf::producer::AsyncProducer;
use log::{error, info, warn};
use futures_util::lock::{Mutex, MutexGuard};
use async_ringbuf::traits::Split;
use futures::channel::oneshot::Receiver;
use futures_util::future::FusedFuture;
use futures_util::select;
use ringbuf::consumer::Consumer;
use crate::{ActorIdentity, ActorName};
use crate::channel_builder::{ChannelBacking, InternalReceiver, InternalSender};
use futures::FutureExt;
use futures_timer::Delay;
use ringbuf::traits::Observer;

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
    node: HashMap<ActorName, Arc<Mutex<(SideChannel,Receiver<()>)>>>,
    pub(crate) backplane: HashMap<ActorName, Arc<Mutex<SideChannel>>>,
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
    /// An `Option` containing an `Arc<Mutex<(SideChannel,Receiver<()>)>>` if the node exists.
    pub fn node_tx_rx(&self, key: ActorName) -> Option<Arc<Mutex<(SideChannel,Receiver<()>) >>> {
                self.node.get(&key).cloned()
    }

    /// Registers a new node with the specified name and capacity.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the node.
    /// * `capacity` - The capacity of the ring buffer.
    pub fn register_node(&mut self, key: ActorName, capacity: usize, shutdown_rx: Receiver<()>) -> bool {
            // Message to the node
            let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
            let (sender_tx, receiver_tx) = rb.split();

            // Response from the node
            let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
            let (sender_rx, receiver_rx) = rb.split();

            if self.node.get(&key).is_some() || self.backplane.get(&key).is_some()  {
                warn!("Node with name {:?} already exists, check suffix usage", key);
                false
            } else {
                self.node.insert(key, Arc::new(Mutex::new(((sender_rx, receiver_tx), shutdown_rx) )));
                self.backplane.insert(key, Arc::new(Mutex::new((sender_tx, receiver_rx))));
                true
            }
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
    pub async fn node_call(&self, msg: Box<dyn Any + Send + Sync>, id: ActorName) -> Option<Box<dyn Any + Send + Sync>> {

        if let Some(sc) = self.backplane.get(&id) {
            let mut sc_guard = sc.lock().await;
            let (tx, rx) = sc_guard.deref_mut();
            match tx.push(msg).await {
                Ok(_) => {
                    //warn!("pushed message to node: {:?}", id);
                    //do nothing else but immediately get the response for more deterministic testing
                    return rx.pop().await;
                }
                Err(e) => {
                    error!("Error sending test implementation request: {:?}", e);
                }
            }
        } else {
            error!("Node with name {:?} not found", id);
        }
        None
    }
}

/// The `SideChannelResponder` struct provides a way to respond to messages from a side channel.
pub struct SideChannelResponder {
    pub(crate) arc: Arc<Mutex<(SideChannel,Receiver<()>)>>,
    pub(crate) identity: ActorIdentity,
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
    pub fn new(arc: Arc<Mutex<(SideChannel,Receiver<()>)>>, identity: ActorIdentity) -> Self {
        SideChannelResponder { arc, identity}
    }

    pub async fn wait_available_units(&mut self, count: usize) -> bool {
        let mut guard = self.arc.lock().await;
        let ((ref mut tx, ref mut rx), ref mut shutdown) = guard.deref_mut();

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
        let ((ref mut tx, ref mut rx),shutdown) = guard.deref_mut();
        if let Some(q) = rx.try_pop() {
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
    use log::info;

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

        let actor = ActorIdentity::new(1,"test_actor",None);

        let (_tx,rx) = oneshot::channel();

        hub.register_node(actor.label.clone(), 10, rx );

        let node = hub.node_tx_rx(actor.label.clone());
        assert!(node.is_some(), "Node should be registered and retrievable");
    }


    #[test]
    async fn test_node_call_success() {

        // Simulates Graph creation where we init the side channel hub
        // and register our actors for future testing
        let mut hub = SideChannelHub::default();
        let actor = ActorIdentity::new(1,"test_actor",None);
        let actor_name = actor.label;
        let text = format!("hub: {:?} actor: {:?}",hub,actor_name);
        info!("custom debug {:?}",text);

        let (_tx,rx) = oneshot::channel();
        assert_eq!(true, hub.register_node(actor_name.clone(), 10, rx));
        let (_tx,rx) = oneshot::channel();
        assert_eq!(false, hub.register_node(actor_name.clone(), 10, rx));


        // Simulates the startup of the actor where we typically lookup the responder once
        let responder_inside_actor = {
            let (tx,rx) = oneshot::channel::<()>();
            let r = Arc::new(Mutex::new(rx));

            let node_tx_rx = hub.node_tx_rx(actor_name);
            if let Some(ref tr) = node_tx_rx {
                Some(SideChannelResponder::new(tr.clone(), actor))
            } else {
                None
            }

        };
        // Simulates an actor which responds when a message is sent
        //this is our simulated actor to respond to our node_call below
        nuclei::spawn_local(async move {
            responder_inside_actor.expect("should exist").respond_with(|msg| {
                let received_msg = msg.downcast_ref::<i32>().unwrap();
                Box::new(received_msg * 2) as Box<dyn Any + Send + Sync>
            }).await;
        }).detach();

        // Simulates the main test block which makes a call and awaits for the response.
        //test code will call to the actor and only return after above actor responds
        let msg = Box::new(42) as Box<dyn Any + Send + Sync>;
        let result = hub.node_call(msg, actor_name).await;

         assert!(result.is_some(), "Should receive a response");
         let r = result.unwrap();
         let response = r.downcast_ref::<i32>().unwrap();
         assert_eq!(*response, 84, "Response should match the expected value");
    }

}
