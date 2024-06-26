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
type NodeName = String;

/// The `SideChannelHub` struct manages side channels for nodes in the graph.
/// Each node holds its own lock on read and write to the backplane.
/// The backplane functions as a central message hub, ensuring that only one user can hold it at a time.
#[derive(Default)]
pub struct SideChannelHub {
    node: HashMap<NodeName, Arc<Mutex<SideChannel>>>,
    pub(crate) backplane: HashMap<NodeName, SideChannel>,
}

impl SideChannelHub {
    /// Creates a new `SideChannelHub`.

    // pub fn new() -> Self {
    //     SideChannelHub {
    //         node: Default::default(),
    //         backplane: HashMap::new(),
    //     }
    // }

    /// Retrieves the transmitter and receiver for a node by its name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the node.
    ///
    /// # Returns
    ///
    /// An `Option` containing an `Arc<Mutex<SideChannel>>` if the node exists.
    pub fn node_tx_rx(&self, name: &str) -> Option<Arc<Mutex<SideChannel>>> {
        self.node.get(name).cloned()
    }

    /// Registers a new node with the specified name and capacity.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the node.
    /// * `capacity` - The capacity of the ring buffer.
    pub fn register_node(&mut self, name: &str, capacity: usize) {
        // Message to the node
        let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
        let (sender_tx, receiver_tx) = rb.split();

        // Response from the node
        let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
        let (sender_rx, receiver_rx) = rb.split();

        self.node.insert(name.into(), Arc::new(Mutex::new((sender_rx, receiver_tx))));
        self.backplane.insert(name.into(), (sender_tx, receiver_rx));
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
    /// # Examples
    ///
    /// ```
    /// use steady_state::graph_testing::SideChannelHub;
    /// let mut hub = SideChannelHub::new();
    /// hub.register_node("test_node", 10);
    /// let response = hub.node_call(Box::new("test message"), "test_node"); //.await
    /// ```
    pub async fn node_call(&mut self, msg: Box<dyn Any + Send + Sync>, name: &str) -> Option<Box<dyn Any + Send + Sync>> {
        if let Some((tx, rx)) = self.backplane.get_mut(name) {
            match tx.push(msg).await {
                Ok(_) => {
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
    pub fn new(arc: Arc<Mutex<SideChannel>>) -> Self {
        SideChannelResponder { arc }
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
        if let Some(msg) = rx.pop().await {
            match tx.push(f(msg)).await {
                Ok(_) => {}
                Err(e) => {
                    error!("Error sending test implementation response: {:?}", e);
                }
            }
        }
    }
}



