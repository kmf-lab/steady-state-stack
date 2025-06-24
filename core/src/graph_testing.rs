//! This module provides utilities for testing graphs in the SteadyState project.
//!
//! It supports side channels for sending and receiving messages from actors in the graph,
//! enabling simulation of real-world scenarios and robust graph testing.

use std::any::Any;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;
use std::ops::DerefMut;
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
use crate::{ActorIdentity, ActorName, Rx, RxBundle, SteadyActor, TxBundle};
use crate::channel_builder::{ChannelBacking, InternalReceiver, InternalSender};
use ringbuf::traits::Observer;
use crate::actor_builder::NodeTxRx;
use crate::steady_actor::SendOutcome;
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use ringbuf::producer::Producer;
use crate::simulate_edge::SimStepResult;

/// Represents the result of a graph test, which can be either success or error.
///
/// Used to encapsulate the outcome of testing a graph of actors within the SteadyState framework.
#[derive(Debug)]
pub enum GraphTestResult<K, E>
where
    K: Any + Send + Sync + Debug,
    E: Any + Send + Sync + Debug,
{
    /// Indicates a successful test result, containing the value `K`.
    Ok(K),
    /// Indicates a failed test result, containing the error value `E`.
    Err(E),
}

/// Manages side channels for nodes in the graph, providing a central message hub.
///
/// Each node holds its own lock on read and write to the backplane.
/// The backplane ensures only one user can hold it at a time.
#[derive(Clone, Default)]
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
pub(crate) type SideChannel =
(InternalSender<Box<dyn Any + Send + Sync>>, InternalReceiver<Box<dyn Any + Send + Sync>>);

/// Marker trait for actions that can be performed on a stage.
pub trait StageAction {}

/// Represents a direction action for a stage, such as echoing a message.
pub enum StageDirection<T> {
    /// Echo a message.
    Echo(T),
    /// Echo a message at a specific index.
    EchoAt(usize, T),
}

/// Represents a wait-for action for a stage, such as waiting for a message.
pub enum StageWaitFor<T: Debug + Eq> {
    /// Wait for a specific message with a timeout.
    Message(T, Duration),
    /// Wait for a specific message at a given index with a timeout.
    MessageAt(usize, T, Duration),
}

impl<T: Debug + Eq> StageAction for StageWaitFor<T> {}
impl<T: Debug + Clone> StageAction for StageDirection<T> {}

impl StageManager {
    /// Retrieves the transmitter and receiver for a node by its id.
    ///
    /// # Arguments
    /// * `key` - The name of the node.
    ///
    /// # Returns
    /// An `Option` containing an `Arc<NodeTxRx>` if the node exists.
    pub(crate) fn node_tx_rx(&self, key: ActorName) -> Option<Arc<NodeTxRx>> {
        self.node.get(&key).cloned()
    }

    /// Registers a new node with the specified name and capacity.
    ///
    /// # Arguments
    /// * `key` - The name of the node.
    /// * `capacity` - The capacity of the ring buffer.
    /// * `shutdown_rx` - The shutdown receiver for the node.
    ///
    /// # Returns
    /// `true` if the node was registered, `false` if it already exists.
    pub(crate) fn register_node(
        &mut self,
        key: ActorName,
        capacity: usize,
        shutdown_rx: Receiver<()>,
    ) -> bool {
        let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
        let (sender_tx, receiver_tx) = rb.split();

        let rb = AsyncRb::<ChannelBacking<Box<dyn Any + Send + Sync>>>::new(capacity);
        let (sender_rx, receiver_rx) = rb.split();

        if self.node.contains_key(&key) || self.backplane.contains_key(&key) {
            warn!("Node with name {:?} already exists, check suffix usage", key);
            false
        } else {
            self.node.insert(
                key,
                Arc::new(Mutex::new(((sender_rx, receiver_tx), shutdown_rx))),
            );
            self.backplane
                .insert(key, Arc::new(Mutex::new((sender_tx, receiver_rx))));
            true
        }
    }

    /// Performs an action on an actor by name.
    ///
    /// # Arguments
    /// * `name` - The name of the actor.
    /// * `action` - The action to perform.
    ///
    /// # Returns
    /// The result of the action as a boxed value or error.
    pub fn actor_perform<S: StageAction + 'static + Send + Sync>(
        &self,
        name: &'static str,
        action: S,
    ) -> Result<Box<dyn Any + Send + Sync>, Box<dyn Error>> {
        self.call_actor_internal(Box::new(action), ActorName::new(name, None))
    }

    /// Performs an action on an actor by name and suffix.
    ///
    /// # Arguments
    /// * `name` - The name of the actor.
    /// * `suffix` - The suffix for the actor name.
    /// * `action` - The action to perform.
    ///
    /// # Returns
    /// The result of the action as a boxed value or error.
    pub fn actor_perform_with_suffix<S: StageAction + 'static + Send + Sync>(
        &self,
        name: &'static str,
        suffix: usize,
        action: S,
    ) -> Result<Box<dyn Any + Send + Sync>, Box<dyn Error>> {
        self.call_actor_internal(Box::new(action), ActorName::new(name, Some(suffix)))
    }

    /// Sends a message to a node and waits for a response.
    ///
    /// # Arguments
    /// * `msg` - The message to send.
    /// * `id` - The name of the node.
    ///
    /// # Returns
    /// The response message or an error.
    pub(crate) fn call_actor_internal(
        &self,
        msg: Box<dyn Any + Send + Sync>,
        id: ActorName,
    ) -> Result<Box<dyn Any + Send + Sync>, Box<dyn Error>> {
        use crate::abstract_executor_async_std::core_exec;

        if let Some(sc) = self.backplane.get(&id) {
            core_exec::block_on(async move {
                let mut sc_guard = sc.lock().await;
                let (tx, rx) = sc_guard.deref_mut();
                match tx.push(msg).await {
                    Ok(_) => {
                        if let Some(response) = rx.pop().await {
                            let is_ok = response
                                .downcast_ref::<&str>()
                                .map(|msg| *msg == OK_MESSAGE)
                                .or_else(|| {
                                    response
                                        .downcast_ref::<String>()
                                        .map(|msg| msg == OK_MESSAGE)
                                })
                                .unwrap_or(false);

                            let is_timeout = response
                                .downcast_ref::<&str>()
                                .map(|msg| *msg == TIMEOUT)
                                .or_else(|| {
                                    response
                                        .downcast_ref::<String>()
                                        .map(|msg| msg == TIMEOUT)
                                })
                                .unwrap_or(false);

                            if is_ok {
                                Ok(response)
                            } else if is_timeout {
                                Err(TIMEOUT.into())
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

/// Provides a way to respond to messages from a side channel.
#[derive(Clone)]
pub struct SideChannelResponder {
    pub(crate) arc: Arc<Mutex<(SideChannel, Receiver<()>)>>,
    pub(crate) identity: ActorIdentity,
}

/// Constant for the "ok" message.
pub(crate) const OK_MESSAGE: &str = "ok";
/// Constant for the "timeout" message.
pub(crate) const TIMEOUT: &str = "timeout, no message";

impl SideChannelResponder {
    /// Simulates a direction action by sending a message to a transmitter.
    ///
    /// # Arguments
    /// * `tx_core` - The transmitter core.
    /// * `actor` - The actor instance.
    /// * `index` - The index for the action.
    ///
    /// # Returns
    /// The simulation step result or error.
    pub fn simulate_direction<
        'a,
        T: 'static + Debug + Clone,
        X: TxCore<MsgIn<'a> = T>,
        C: SteadyActor,
    >(
        &self,
        tx_core: &mut X,
        actor: &mut C,
        index: usize,
    ) -> Result<SimStepResult, Box<dyn Error>>
    where
        <X as TxCore>::MsgOut: Send,
        <X as TxCore>::MsgOut: Sync,
        <X as TxCore>::MsgOut: 'static,
    {
        let r = self.respond_with(
            move |message, actor| match message.downcast_ref::<StageDirection<X::MsgIn<'a>>>() {
                Some(msg) => match msg {
                    StageDirection::Echo(m) => match actor.try_send(tx_core, m.clone()) {
                        SendOutcome::Success => Some(Box::new(OK_MESSAGE)),
                        SendOutcome::Blocked(msg) => Some(Box::new(msg)),
                    },
                    StageDirection::EchoAt(i, m) => {
                        if *i == index {
                            match actor.try_send(tx_core, m.clone()) {
                                SendOutcome::Success => Some(Box::new(OK_MESSAGE)),
                                SendOutcome::Blocked(msg) => Some(Box::new(msg)),
                            }
                        } else {
                            None
                        }
                    }
                },
                None => {
                    error!(
                        "direction Unable to cast stage direction to target type: {}",
                        std::any::type_name::<StageDirection<X::MsgIn<'a>>>()
                    );
                    None
                }
            },
            actor,
        );

        match r {
            Ok(true) => Ok(SimStepResult::DidWork),
            Ok(false) => Ok(SimStepResult::NoWork),
            Err(e) => Err(format!("error: {:?}", e).into()),
        }
    }

    /// Simulates waiting for a message on a receiver.
    ///
    /// # Arguments
    /// * `rx_core` - The receiver core.
    /// * `actor` - The actor instance.
    /// * `index` - The index for the action.
    /// * `run_duration` - The duration to wait.
    ///
    /// # Returns
    /// The simulation step result or error.
    pub fn simulate_wait_for<
        T: Debug + Eq + 'static,
        X: RxCore<MsgOut = T>,
        C: SteadyActor,
    >(
        &self,
        rx_core: &mut X,
        actor: &mut C,
        index: usize,
        run_duration: Duration,
    ) -> Result<SimStepResult, Box<dyn Error>>
    where
        <X as RxCore>::MsgOut: std::fmt::Debug,
    {
        let r = self.respond_with(
            move |message, actor_guard| {
                let wait_for: &StageWaitFor<T> = message.downcast_ref::<StageWaitFor<X::MsgOut>>()
                    .unwrap_or_else(|| {
                        panic!(
                            "Unable to take message and downcast it to: {}",
                            std::any::type_name::<T>()
                        )
                    });

                let message = match wait_for {
                    StageWaitFor::Message(m, t) => Some((m, t)),
                    StageWaitFor::MessageAt(i, m, t) => {
                        if *i == index {
                            Some((m, t))
                        } else {
                            None
                        }
                    }
                };

                if let Some((expected, timeout)) = message {
                    match actor_guard.try_take(rx_core) {
                        Some(measured) => {
                            if expected.eq(&measured) {
                                Some(Box::new(OK_MESSAGE))
                            } else {
                                let failure = format!("no match {:?} {:?}", expected, measured);
                                Some(Box::new(failure))
                            }
                        }
                        None => {
                            if run_duration.gt(timeout) {
                                error!("timeout: {:?}", timeout);
                                Some(Box::new(TIMEOUT.to_string()))
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    None
                }
            },
            actor,
        );
        match r {
            Ok(true) => Ok(SimStepResult::DidWork),
            Ok(false) => Ok(SimStepResult::NoWork),
            Err(e) => Err(format!("error: {:?}", e).into()),
        }
    }

    /// Creates a new `SideChannelResponder`.
    ///
    /// # Arguments
    /// * `arc` - The side channel and shutdown receiver.
    /// * `identity` - The actor identity.
    ///
    /// # Returns
    /// A new `SideChannelResponder` instance.
    pub fn new(
        arc: Arc<Mutex<(SideChannel, Receiver<()>)>>,
        identity: ActorIdentity,
    ) -> Self {
        SideChannelResponder { arc, identity }
    }

    /// Waits for a specified number of requests to be available.
    ///
    /// # Arguments
    /// * `count` - The number of requests to wait for.
    ///
    /// # Returns
    /// `true` if the count is met, `false` if shutdown is in process.
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

    /// Listens for a message and echoes it to all outgoing channels in a bundle.
    ///
    /// # Arguments
    /// * `actor` - The actor instance.
    /// * `target_tx_bundle` - The bundle of outgoing channels.
    ///
    /// # Returns
    /// `true` if the operation succeeded, `false` otherwise.
    pub async fn echo_responder_bundle<
        M: 'static + Clone + Debug + Send,
        C: SteadyActor,
    >(
        &self,
        actor: &mut C,
        target_tx_bundle: &mut TxBundle<'_, M>,
    ) -> Result<bool, Box<dyn Error>> {
        if let Some(true) = self.should_apply::<M>().await {
            let girth = target_tx_bundle.len();

            for t in target_tx_bundle.iter_mut() {
                if !actor.wait_vacant(&mut *t, 1).await {
                    return Ok(true);
                }
            }

            self.respond_with(
                |message, actor| {
                    let msg = message.downcast_ref::<M>().expect("error casting");
                    let total: usize = (0..girth)
                        .filter(|&c| {
                            actor.try_send(&mut target_tx_bundle[c], msg.clone()).is_sent()
                        })
                        .count();

                    if total == girth {
                        Some(Box::new("ok".to_string()))
                    } else {
                        let failure =
                            format!("failed to echo to {:?} channels", girth - total);
                        Some(Box::new(failure))
                    }
                },
                actor,
            )
        } else {
            Ok(false)
        }
    }

    /// Verifies a message matches all incoming messages from a bundle of channels.
    ///
    /// # Arguments
    /// * `actor` - The actor instance.
    /// * `source_rx` - The bundle of incoming channels.
    ///
    /// # Returns
    /// `true` if the operation succeeded, `false` otherwise.
    pub async fn equals_responder_bundle<
        M: 'static + Clone + Debug + Send + Eq,
        C: SteadyActor,
    >(
        &self,
        actor: &mut C,
        source_rx: &mut RxBundle<'_, M>,
    ) -> Result<bool, Box<dyn Error>> {
        if let Some(true) = self.should_apply::<M>().await {
            let girth = source_rx.len();

            for x in 0..girth {
                let srx: &mut MutexGuard<Rx<M>> = &mut source_rx[x];
                if !actor.wait_avail(srx, 1).await {
                    return Ok(true);
                }
            }

            self.respond_with(
                |message, actor| {
                    let msg: &M = message.downcast_ref::<M>().expect("error casting");
                    let total = (0..girth)
                        .filter_map(|c| actor.try_take(&mut source_rx[c]))
                        .filter(|m| m.eq(msg))
                        .count();

                    if girth == total {
                        Some(Box::new("ok".to_string()))
                    } else {
                        let failure =
                            format!("match failure {:?} of {:?}", msg, girth - total);
                        Some(Box::new(failure))
                    }
                },
                actor,
            )
        } else {
            Ok(false)
        }
    }

    /// Checks if the next message in the queue matches the expected type without consuming it.
    ///
    /// # Type Parameters
    /// * `M` - The expected message type.
    ///
    /// # Returns
    /// `Some(true)` if the type matches, `Some(false)` if not, or `None` if no message is available.
    pub async fn should_apply<M: 'static>(&self) -> Option<bool> {
        let mut guard = self.arc.lock().await;
        let ((_, rx), _) = guard.deref_mut();

        if let Some(q) = rx.try_peek() {
            let is_correct_type = q.is::<M>();
            if !is_correct_type {
                error!("Fix the unit test sent message, Type must match channel of the target actor but it does not.");
            }
            Some(is_correct_type)
        } else {
            None
        }
    }

    /// Waits until at least one message is available to process.
    pub async fn wait_avail(&self) {
        let mut guard = self.arc.lock().await;
        let ((_, rx), _) = guard.deref_mut();

        rx.wait_occupied(1).await;
    }

    /// Returns the number of available messages in the queue.
    pub fn avail(&self) -> usize {
        let mut guard = self.arc.try_lock().expect("internal lock issue");
        let ((_, rx), _) = guard.deref_mut();
        rx.occupied_len()
    }

    /// Responds to messages from the side channel using the provided function.
    ///
    /// # Arguments
    /// * `f` - A function that takes a message and returns a response.
    /// * `actor` - The actor instance.
    ///
    /// # Returns
    /// `true` if a message was handled, `false` if not, or an error.
    pub fn respond_with<F, C>(
        &self,
        mut f: F,
        actor: &mut C,
    ) -> Result<bool, Box<dyn Error>>
    where
        C: SteadyActor,
        F: FnMut(&Box<dyn Any + Send + Sync>, &mut C) -> Option<Box<dyn Any + Send + Sync>>,
    {
        let mut guard = self.arc.try_lock().expect("internal lock error, should probably try again");
        let ((tx, rx), _shutdown) = guard.deref_mut();

        if rx.is_empty() {
            return Ok(true);
        }
        if let Some(q) = rx.try_peek() {
            if let Some(r) = f(q, actor) {
                match tx.try_push(r) {
                    Ok(_) => {
                        let _ = rx.try_pop();
                        Ok(true)
                    }
                    Err(e) => {
                        error!(
                            "Error sending test implementation response: {:?} Identity: {:?}",
                            e, self.identity
                        );
                        Err("internal error pushing response".into())
                    }
                }
            } else {
                Ok(false)
            }
        } else {
            Ok(true)
        }
    }
}