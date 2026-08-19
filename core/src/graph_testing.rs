//! This module provides utilities for testing graphs in the SteadyState project.
//!
//! It supports side channels for sending and receiving messages from actors in the graph,
//! enabling simulation of real-world scenarios and robust graph testing.

// ss[related testing.graph-for-testing]
use std::any::Any;
// ss[related testing.graph-for-testing]
use std::collections::HashMap;
// ss[related testing.graph-for-testing]
use std::error::Error;
// ss[related testing.graph-for-testing]
use std::fmt::Debug;
// ss[related testing.graph-for-testing]
use std::ops::DerefMut;
// ss[related testing.graph-for-testing]
use std::sync::Arc;
// ss[related testing.graph-for-testing]
use std::sync::Mutex as StdMutex;
// ss[related testing.graph-for-testing]
use std::time::{Duration, Instant};
// ss[related testing.graph-for-testing]
use async_ringbuf::AsyncRb;
// ss[related testing.graph-for-testing]
use async_ringbuf::consumer::AsyncConsumer;
// ss[related testing.graph-for-testing]
use async_ringbuf::producer::AsyncProducer;
// ss[related testing.graph-for-testing]
use log::*;
// ss[related testing.graph-for-testing]
use futures_util::lock::{Mutex, MutexGuard};
// ss[related testing.graph-for-testing]
use async_ringbuf::traits::Split;
// ss[related testing.graph-for-testing]
use futures::channel::oneshot::Receiver;
// ss[related testing.graph-for-testing]
use futures_util::future::FusedFuture;
// ss[related testing.graph-for-testing]
use futures_util::select;
// ss[related testing.graph-for-testing]
use ringbuf::consumer::Consumer;
// ss[related testing.graph-for-testing]
use crate::{ActorIdentity, ActorName, Rx, RxBundle, SteadyActor, TxBundle};
// ss[related testing.graph-for-testing]
use crate::channel_builder::{ChannelBacking, InternalReceiver, InternalSender};
// ss[related testing.graph-for-testing]
use ringbuf::traits::Observer;
// ss[related testing.graph-for-testing]
use crate::actor_builder::NodeTxRx;
// ss[related testing.graph-for-testing]
use crate::steady_actor::SendOutcome;
// ss[related testing.graph-for-testing]
use crate::core_rx::RxCore;
// ss[related testing.graph-for-testing]
use crate::core_tx::TxCore;
// ss[related testing.graph-for-testing]
use ringbuf::producer::Producer;
// ss[related testing.graph-for-testing]
use crate::simulate_edge::SimStepResult;

/// Represents the result of a graph test, which can be either success or error.
///
/// Used to encapsulate the outcome of testing a graph of actors within the SteadyState framework.
#[derive(Debug)]
// ss[related testing.graph-for-testing]
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
// ss[impl testing.stage-manager-integration]
pub struct StageManager {
    node: HashMap<ActorName, Arc<NodeTxRx>>,
    // ss[related testing.graph-for-testing]
    pub(crate) backplane: HashMap<ActorName, Arc<Mutex<SideChannel>>>,
}

// ss[related testing.graph-for-testing]
impl Debug for StageManager {
    // ss[related testing.graph-for-testing]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SideChannelHub")
            .field("node", &self.node)
            .finish()
    }
}

/// Type alias for a side channel, which is a pair of internal sender and receiver.
// ss[related testing.graph-for-testing]
pub(crate) type SideChannel =
(InternalSender<Box<dyn Any + Send + Sync>>, InternalReceiver<Box<dyn Any + Send + Sync>>);

/// Marker trait for actions that can be performed on a stage.
// ss[related testing.graph-for-testing]
pub trait StageAction {}

/// Represents a direction action for a stage, such as echoing a message.
// ss[related testing.graph-for-testing]
pub enum StageDirection<T> {
    /// Echo a message.
    Echo(T),
    /// Echo a message at a specific index.
    EchoAt(usize, T),
}

/// Represents a wait-for action for a stage, such as waiting for a message.
// ss[related testing.graph-for-testing]
pub enum StageWaitFor<T: Debug + Eq> {
    /// Wait for a specific message with a timeout.
    Message(T, Duration),
    /// Wait for a specific message at a given index with a timeout.
    MessageAt(usize, T, Duration),
}

// ss[related testing.graph-for-testing]
impl<T: Debug + Eq> StageAction for StageWaitFor<T> {}
// ss[related testing.graph-for-testing]
impl<T: Debug + Clone> StageAction for StageDirection<T> {}

// ss[related testing.graph-for-testing]
impl StageManager {
    /// Retrieves the transmitter and receiver for a node by its id.
    ///
    /// # Arguments
    /// * `key` - The name of the node.
    ///
    /// # Returns
    /// An `Option` containing an `Arc<NodeTxRx>` if the node exists.
    // ss[related testing.graph-for-testing]
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
    // ss[related testing.graph-for-testing]
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
            trace!(
                "Node with name {:?} already exists, check suffix usage",
                key
            );
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
    // ss[related testing.graph-for-testing]
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
    // ss[related testing.graph-for-testing]
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
    // ss[related testing.graph-for-testing]
    pub(crate) fn call_actor_internal(
        &self,
        msg: Box<dyn Any + Send + Sync>,
        id: ActorName,
    ) -> Result<Box<dyn Any + Send + Sync>, Box<dyn Error>> {
        // ss[related testing.graph-for-testing]
        use crate::core_exec;

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
// ss[related testing.graph-for-testing]
pub struct SideChannelResponder {
    // ss[related testing.graph-for-testing]
    pub(crate) arc: Arc<Mutex<(SideChannel, Receiver<()>)>>,
    // ss[related testing.graph-for-testing]
    pub(crate) identity: ActorIdentity,
    /// Wall-clock start of the current [`StageWaitFor`] command (peeked, not yet completed).
    // ss[related testing.graph-for-testing]
    pub(crate) stage_wait_start: Arc<StdMutex<Option<Instant>>>,
}

/// Constant for the "ok" message.
// ss[related testing.graph-for-testing]
pub(crate) const OK_MESSAGE: &str = "ok";
/// Constant for the "timeout" message.
// ss[related testing.graph-for-testing]
pub(crate) const TIMEOUT: &str = "timeout, no message";

// ss[related testing.graph-for-testing]
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
    // ss[related testing.graph-for-testing]
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
        let wait_reset = self.stage_wait_start.clone();
        let r = self.respond_with(
            move |message, actor| {
                let Some(msg) = message.downcast_ref::<StageDirection<X::MsgIn<'a>>>() else {
                    return None;
                };
                match msg {
                    StageDirection::Echo(m) => match actor.try_send(tx_core, m.clone()) {
                        SendOutcome::Success => {
                            if let Ok(mut w) = wait_reset.lock() {
                                *w = None;
                            }
                            Some(Box::new(OK_MESSAGE))
                        }
                        SendOutcome::Blocked(msg) | SendOutcome::Timeout(msg) | SendOutcome::Closed(msg) => Some(Box::new(msg)),
                    },
                    StageDirection::EchoAt(i, m) => {
                        if *i == index {
                            match actor.try_send(tx_core, m.clone()) {
                                SendOutcome::Success => {
                                    if let Ok(mut w) = wait_reset.lock() {
                                        *w = None;
                                    }
                                    Some(Box::new(OK_MESSAGE))
                                }
                                SendOutcome::Blocked(msg) | SendOutcome::Timeout(msg) | SendOutcome::Closed(msg) => Some(Box::new(msg)),
                            }
                        } else {
                            None
                        }
                    }
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
    ///
    /// # Returns
    /// The simulation step result or error.
    // ss[related testing.graph-for-testing]
    pub fn simulate_wait_for<
        T: Debug + Eq + 'static,
        X: RxCore<MsgOut = T>,
        C: SteadyActor,
    >(
        &self,
        rx_core: &mut X,
        actor: &mut C,
        index: usize,
    ) -> Result<SimStepResult, Box<dyn Error>>
    where
        <X as RxCore>::MsgOut: std::fmt::Debug,
    {
        let wait_clock = self.stage_wait_start.clone();
        let r = self.respond_with(
            move |message, actor_guard| {
                let Some(wait_for) = message.downcast_ref::<StageWaitFor<X::MsgOut>>() else {
                    return None;
                };

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
                            if let Ok(mut w) = wait_clock.lock() {
                                *w = None;
                            }
                            if expected.eq(&measured) {
                                Some(Box::new(OK_MESSAGE))
                            } else {
                                let failure = format!("no match {:?} {:?}", expected, measured);
                                Some(Box::new(failure))
                            }
                        }
                        None => {
                            let mut g = wait_clock.lock().ok()?;
                            match *g {
                                None => {
                                    *g = Some(Instant::now());
                                    None
                                }
                                Some(start) => {
                                    if start.elapsed() >= *timeout {
                                        *g = None;
                                        error!("timeout: {:?}", timeout);
                                        Some(Box::new(TIMEOUT.to_string()))
                                    } else {
                                        None
                                    }
                                }
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
    // ss[related testing.graph-for-testing]
    pub fn new(
        arc: Arc<Mutex<(SideChannel, Receiver<()>)>>,
        identity: ActorIdentity,
    ) -> Self {
        SideChannelResponder {
            arc,
            identity,
            stage_wait_start: Arc::new(StdMutex::new(None)),
        }
    }

    /// Waits for a specified number of requests to be available.
    ///
    /// # Arguments
    /// * `count` - The number of requests to wait for.
    ///
    /// # Returns
    /// `true` if the count is met, `false` if shutdown is in process.
    // ss[related testing.graph-for-testing]
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
    // ss[related testing.graph-for-testing]
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
    // ss[related testing.graph-for-testing]
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
    // ss[related testing.graph-for-testing]
    pub async fn should_apply<M: 'static>(&self) -> Option<bool> {
        let mut guard = self.arc.lock().await;
        let ((_, rx), _) = guard.deref_mut();

        if let Some(q) = rx.try_peek() {
            let is_correct_type = q.is::<M>();
            if !is_correct_type {
                debug!(
                    "should_apply: message at channel head does not match the requested type parameter"
                );
            }
            Some(is_correct_type)
        } else {
            None
        }
    }

    /// Waits until at least one message is available to process.
    // ss[related testing.graph-for-testing]
    pub async fn wait_avail(&self) {
        let mut guard = self.arc.lock().await;
        let ((_, rx), _) = guard.deref_mut();

        rx.wait_occupied(1).await;
    }

    /// Returns the number of available messages in the queue.
    // ss[related testing.graph-for-testing]
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
    /// `true` if a message was handled and a response was sent to the test thread,
    /// `false` if there was nothing to do or the handler could not complete yet, or an error.
    // ss[related testing.graph-for-testing]
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
            return Ok(false);
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
            Ok(false)
        }
    }
}

#[cfg(test)]
// ss[related testing.graph-for-testing]
mod graph_testing_tests {
    // ss[related testing.graph-for-testing]
    use super::*;
    // ss[related testing.graph-for-testing]
    use std::error::Error;
    // ss[related testing.graph-for-testing]
    use std::time::Duration;
    // ss[related testing.graph-for-testing]
    use aeron::aeron::Aeron;
    // ss[related testing.graph-for-testing]
    use futures::channel::oneshot;
    // ss[related testing.graph-for-testing]
    use crate::*;
    // ss[related testing.graph-for-testing]
    use crate::ActorName;
    // ss[related testing.graph-for-testing]
    use crate::ActorIdentity;
    // ss[related testing.graph-for-testing]
    use crate::distributed::aqueduct_stream::Defrag;
    // ss[related testing.graph-for-testing]
    use crate::simulate_edge::IntoSimRunner;
    // ss[related testing.graph-for-testing]
    use crate::channel_builder::ChannelBuilder;
    // ss[related testing.graph-for-testing]
    use crate::RxCoreBundle;
    // ss[related testing.graph-for-testing]
    use crate::steady_actor::BlockingCallFuture;
    // ss[related testing.graph-for-testing]
    use crate::TxCoreBundle;

    // ss[related testing.graph-for-testing]
    struct DummyActor {
        has_data: bool,
    }

    // ss[related testing.graph-for-testing]
    impl SteadyActor for DummyActor {
        // ss[related testing.graph-for-testing]
        fn frame_rate_ms(&self) -> u64 { 0 }
        // ss[related testing.graph-for-testing]
        fn regeneration(&self) -> u32 { 0 }
        // ss[related testing.graph-for-testing]
        fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> { None }
        // ss[related testing.graph-for-testing]
        async fn simulated_behavior(self, _sims: Vec<&dyn IntoSimRunner<Self>>) -> Result<(), Box<dyn Error>> { Ok(()) }
        // ss[related testing.graph-for-testing]
        fn loglevel(&self, _loglevel: crate::LogLevel) {}
        // ss[related testing.graph-for-testing]
        fn relay_stats_smartly(&mut self) -> bool { false }
        // ss[related testing.graph-for-testing]
        fn relay_stats(&mut self) {}
        // ss[related testing.graph-for-testing]
        async fn relay_stats_periodic(&mut self, _duration_rate: Duration) -> bool { false }
        // ss[related testing.graph-for-testing]
        fn is_liveliness_in(&self, _target: &[GraphLivelinessState]) -> bool { false }
        // ss[related testing.graph-for-testing]
        fn is_liveliness_building(&self) -> bool { false }
        // ss[related testing.graph-for-testing]
        fn is_liveliness_running(&self) -> bool { false }
        // ss[related testing.graph-for-testing]
        fn is_liveliness_stop_requested(&self) -> bool { false }
        // ss[related testing.graph-for-testing]
        fn is_liveliness_shutdown_timeout(&self) -> Option<Duration> { None }
        // ss[related testing.graph-for-testing]
        fn flush_defrag_messages<S: StreamControlItem>(
            &mut self,
            _item: &mut Tx<S>,
            _data: &mut Tx<u8>,
            _defrag: &mut Defrag<S>,
        ) -> (u32, u32, Option<i32>) { (0, 0, None) }
        // ss[related testing.graph-for-testing]
        async fn wait_periodic(&self, _duration_rate: Duration) -> bool { false }
        // ss[related testing.graph-for-testing]
        async fn wait_timeout(&self, _timeout: Duration) -> bool { false }
        // ss[related testing.graph-for-testing]
        async fn wait(&self, _duration: Duration) {}
        // ss[related testing.graph-for-testing]
        async fn wait_avail<T: RxCore>(&self, _this: &mut T, _size: usize) -> bool { true }
        // ss[related testing.graph-for-testing]
        async fn wait_avail_bundle<T: RxCore>(
            &self,
            _this: &mut RxCoreBundle<'_, T>,
            _size: usize,
            _ready_channels: usize,
        ) -> bool { true }
        // ss[related testing.graph-for-testing]
        async fn wait_avail_index<T: RxCore>(
            &self,
            _this: &mut RxCoreBundle<'_, T>,
            _counts: &[usize],
        ) -> Option<usize> { Some(0) }
        // ss[related testing.graph-for-testing]
        async fn wait_future_void<F>(&self, _fut: F) -> bool where F: FusedFuture<Output = ()> + 'static + Send + Sync { false }
        // ss[related testing.graph-for-testing]
        async fn wait_vacant<T: TxCore>(&self, _this: &mut T, _count: T::MsgSize) -> bool { true }
        // ss[related testing.graph-for-testing]
        async fn wait_vacant_bundle<T: TxCore>(
            &self,
            _this: &mut TxCoreBundle<'_, T>,
            _count: T::MsgSize,
            _ready_channels: usize,
        ) -> bool { true }
        // ss[related testing.graph-for-testing]
        async fn wait_vacant_index<T: TxCore>(
            &self,
            _this: &mut TxCoreBundle<'_, T>,
            _counts: &[T::MsgSize],
        ) -> Option<usize> { Some(0) }
        // ss[related testing.graph-for-testing]
        async fn wait_avail_vacant_index<R: RxCore, T: TxCore>(
            &self,
            _rx: &mut RxCoreBundle<'_, R>,
            _tx: &mut TxCoreBundle<'_, T>,
            _avail_counts: &[usize],
            _vacant_counts: &[T::MsgSize],
        ) -> Option<usize> { Some(0) }
        // ss[related testing.graph-for-testing]
        async fn wait_shutdown(&self) -> bool { false }
        // ss[related testing.graph-for-testing]
        fn peek_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceSource<'b> where T: RxCore { unimplemented!() }
        // ss[related testing.graph-for-testing]
        fn advance_take_index<T: RxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> RxDone { unimplemented!() }
        // ss[related testing.graph-for-testing]
        fn take_slice<T: RxCore>(
            &mut self,
            _this: &mut T,
            _target: T::SliceTarget<'_>,
        ) -> RxDone where T::MsgItem: Copy { unimplemented!() }
        // ss[related testing.graph-for-testing]
        fn send_slice<T: TxCore>(
            &mut self,
            _this: &mut T,
            _source: T::SliceSource<'_>,
        ) -> TxDone where T::MsgOut: Copy { unimplemented!() }
        // ss[related testing.graph-for-testing]
        fn poke_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceTarget<'b> where T: TxCore { unimplemented!() }
        // ss[related testing.graph-for-testing]
        fn advance_send_index<T: TxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> TxDone { unimplemented!() }
        // ss[related testing.graph-for-testing]
        fn try_peek<'a, T>(&'a self, _this: &'a mut Rx<T>) -> Option<&'a T> { None }
        // ss[related testing.graph-for-testing]
        fn try_peek_iter<'a, T>(
            &'a self,
            _this: &'a mut Rx<T>,
        ) -> impl Iterator<Item = &'a T> + 'a { std::iter::empty() }
        // ss[related testing.graph-for-testing]
        fn is_empty<T: RxCore>(&self, _this: &mut T) -> bool { !self.has_data }
        // ss[related testing.graph-for-testing]
        fn avail_units<T: RxCore>(&self, this: &mut T) -> T::MsgSize { if self.has_data { this.one() } else { unimplemented!() } }
        // ss[related testing.graph-for-testing]
        async fn peek_async<'a, T: RxCore>(
            &'a self,
            _this: &'a mut T,
        ) -> Option<T::MsgPeek<'a>> { None }
        // ss[related testing.graph-for-testing]
        fn send_iter_until_full<T, I: Iterator<Item = T>>(
            &mut self,
            _this: &mut Tx<T>,
            _iter: I,
        ) -> usize { 0 }
        // ss[related testing.graph-for-testing]
        fn try_send<T: TxCore>(
            &mut self,
            this: &mut T,
            msg: T::MsgIn<'_>,
        ) -> SendOutcome<T::MsgOut> {
            if self.has_data {
                match this.shared_try_send(msg) {
                    Ok(_) => SendOutcome::Success,
                    Err(blocked) => SendOutcome::Blocked(blocked),
                }
            } else {
                SendOutcome::Success
            }
        }
        // ss[related testing.graph-for-testing]
        fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> {
            if self.has_data {
                this.shared_try_take().map(|(_done, msg)| msg)
            } else {
                None
            }
        }
        // ss[related testing.graph-for-testing]
        fn is_full<T: TxCore>(&self, _this: &mut T) -> bool { false }
        // ss[related testing.graph-for-testing]
        fn vacant_units<T: TxCore>(&self, this: &mut T) -> T::MsgSize { this.one() }
        // ss[related testing.graph-for-testing]
        async fn wait_empty<T: TxCore>(&self, _this: &mut T) -> bool { false }
        // ss[related testing.graph-for-testing]
        fn take_into_iter<'a, T: Sync + Send>(
            &mut self,
            _this: &'a mut Rx<T>,
        ) -> impl Iterator<Item = T> + 'a { std::iter::empty() }
        // ss[related testing.graph-for-testing]
        async fn call_async<F>(&self, _operation: F) -> Option<F::Output> where F: Future { None }
        // ss[related testing.graph-for-testing]
        fn call_blocking<F, T>(&self, f: F) -> BlockingCallFuture<T>
        where
            F: FnOnce() -> T + Send + 'static,
            T: Send + 'static {
            BlockingCallFuture(core_exec::spawn_blocking(f))
        }
        // ss[related testing.graph-for-testing]
        async fn send_async<T: TxCore>(
            &mut self,
            _this: &mut T,
            _a: T::MsgIn<'_>,
            _saturation: SendSaturation,
        ) -> SendOutcome<T::MsgOut> { SendOutcome::Success }
        // ss[related testing.graph-for-testing]
        async fn take_async<T>(&mut self, _this: &mut Rx<T>) -> Option<T> { None }
        // ss[related testing.graph-for-testing]
        async fn take_async_with_timeout<T>(
            &mut self,
            _this: &mut Rx<T>,
            _timeout: Duration,
        ) -> Option<T> { None }
        // ss[related testing.graph-for-testing]
        async fn yield_now(&self) {}
        // ss[related testing.graph-for-testing]
        fn sidechannel_responder(&self) -> Option<SideChannelResponder> { None }
        // ss[related testing.graph-for-testing]
        fn is_running<F: FnMut() -> bool>(&mut self, _accept_fn: F) -> bool { true }
        // ss[related testing.graph-for-testing]
        async fn request_shutdown(&mut self) {}
        // ss[related testing.graph-for-testing]
        fn args<A: Any>(&self) -> Option<&A> { None }
        // ss[related testing.graph-for-testing]
        fn identity(&self) -> ActorIdentity { ActorIdentity::default() }
        // ss[related testing.graph-for-testing]
        fn is_showstopper<T>(&self, _rx: &mut Rx<T>, _threshold: usize) -> bool { false }

        // ss[related testing.graph-for-testing]
        fn set_dot_display_text(&mut self, _text: Option<&str>) {}
    }

    // ss[verify testing.graph-for-testing]
    // ss[verify testing.graph-for-testing]
    // ss[verify testing.mock-main-thread]
    // ss[verify testing.deterministic-no-sleep]
    #[test]
    // ss[related testing.graph-for-testing]
    fn test_graph_test_result() -> Result<(), Box<dyn Error>> {
        let ok: GraphTestResult<i32, String> = GraphTestResult::Ok(42);
        if let GraphTestResult::Ok(val) = ok {
            assert_eq!(val, 42);
        } else {
            return Err("Expected Ok".into());
        }

        let err: GraphTestResult<i32, String> = GraphTestResult::Err("error".to_string());
        if let GraphTestResult::Err(val) = err {
            assert_eq!(val, "error");
        } else {
            return Err("Expected Err".into());
        }

        Ok(())
    }

    // ss[verify testing.stage-manager-integration]
    #[test]
    // ss[related testing.graph-for-testing]
    fn test_stack_guarded_graph() -> Result<(), Box<dyn Error>> {
        SteadyRunner::test_build()
            .with_stack_size(16 * 1024 * 1024)
            .run((), |mut graph| {
                graph.start();
                let sm = graph.stage_manager();
                sm.final_bow();
                graph.request_shutdown();
                graph.block_until_stopped(Duration::from_secs(5))
            })
    }

    // ss[verify testing.stage-manager-integration]
    #[test]
    // ss[related testing.graph-for-testing]
    fn test_stage_manager_default() -> Result<(), Box<dyn Error>> {
        let manager = StageManager::default();
        assert!(manager.node.is_empty());
        assert!(manager.backplane.is_empty());
        Ok(())
    }

    // ss[verify testing.stage-manager-integration]
    #[test]
    // ss[related testing.graph-for-testing]
    fn test_stage_manager_clone() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("test", None), 10, shutdown_rx);

        let cloned = manager.clone();
        assert_eq!(manager.node.len(), cloned.node.len());
        assert_eq!(manager.backplane.len(), cloned.backplane.len());
        Ok(())
    }

    // ss[verify testing.stage-manager-integration]
    #[test]
    // ss[related testing.graph-for-testing]
    fn test_stage_manager_debug() -> Result<(), Box<dyn Error>> {
        let manager = StageManager::default();
        let debug_str = format!("{:?}", manager);
        assert!(debug_str.contains("SideChannelHub"));
        Ok(())
    }

    #[test]
    // ss[verify testing.graph-for-testing]
    fn test_node_tx_rx() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("test", None), 10, shutdown_rx);

        let node = manager.node_tx_rx(ActorName::new("test", None));
        assert!(node.is_some());

        let missing = manager.node_tx_rx(ActorName::new("missing", None));
        assert!(missing.is_none());
        Ok(())
    }

    #[test]
    // ss[verify testing.graph-for-testing]
    fn test_register_node() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();

        let success = manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
        assert!(success);
        assert_eq!(manager.node.len(), 1);
        assert_eq!(manager.backplane.len(), 1);

        let (_shutdown_tx2, shutdown_rx2) = oneshot::channel();
        let duplicate = manager.register_node(ActorName::new("test", None), 10, shutdown_rx2);
        assert!(!duplicate);
        Ok(())
    }

    #[test]
    // ss[verify testing.graph-for-testing]
    fn test_call_actor_internal_errors() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let name = ActorName::new("test", None);
        manager.register_node(name, 1, shutdown_rx);

        // Correct simulation: Use the NODE side to simulate the actor
        let node_side = manager.node_tx_rx(name).unwrap();
        core_exec::spawn_detached(async move {
            let mut guard = node_side.lock().await;
            let ((tx_prod, _), _) = guard.deref_mut();
            // Wait for request and send malformed response
            let _ = tx_prod.try_push(Box::new(42i32)); 
        });

        let res = manager.call_actor_internal(Box::new("req"), name);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("unexpected message"));
        Ok(())
    }

    #[test]
    // ss[verify testing.graph-for-testing]
    fn test_side_channel_responder_new() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        assert_eq!(responder.identity, ActorIdentity::default());
        Ok(())
    }

    // ss[verify testing.deterministic-no-sleep]
    #[test]
    // ss[related testing.graph-for-testing]
    fn test_avail() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let backplane = manager.backplane.get(&ActorName::new("test", None)).unwrap().clone();

        assert_eq!(responder.avail(), 0);

        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (tx, _) = guard.deref_mut();
            tx.push(Box::new(42)).await
        }).expect("");

        assert_eq!(responder.avail(), 1);
        Ok(())
    }

    #[test]
    // ss[verify testing.graph-for-testing]
    fn test_should_apply_logic() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        let backplane = manager.backplane.get(&ActorName::new("test", None)).unwrap().clone();

        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (tx, _) = guard.deref_mut();
            tx.push(Box::new(42i32)).await
        }).expect("");

        let result = core_exec::block_on(responder.should_apply::<i32>());
        assert_eq!(result, Some(true));

        let result_wrong = core_exec::block_on(responder.should_apply::<String>());
        assert_eq!(result_wrong, Some(false));
        Ok(())
    }

    // ss[related testing.graph-for-testing]
    async fn pipeline_generator_edge(
        actor: SteadyActorShadow,
        tx: SteadyTx<u64>,
    ) -> Result<(), Box<dyn Error>> {
        let actor = actor.into_spotlight([], [&tx]);
        // ss[related actor.internal-behavior-logic]
        if actor.use_internal_behavior {
            Ok(())
        } else {
            actor.simulated_behavior(sim_runners!(tx)).await
        }
    }

    // ss[related testing.graph-for-testing]
    async fn pipeline_heartbeat_edge(
        actor: SteadyActorShadow,
        tx: SteadyTx<u64>,
    ) -> Result<(), Box<dyn Error>> {
        let actor = actor.into_spotlight([], [&tx]);
        if actor.use_internal_behavior {
            Ok(())
        } else {
            actor.simulated_behavior(sim_runners!(tx)).await
        }
    }

    // ss[related testing.graph-for-testing]
    async fn pipeline_logger_edge(
        actor: SteadyActorShadow,
        rx: SteadyRx<u64>,
    ) -> Result<(), Box<dyn Error>> {
        let actor = actor.into_spotlight([&rx], []);
        if actor.use_internal_behavior {
            Ok(())
        } else {
            actor.simulated_behavior(sim_runners!(rx)).await
        }
    }

    // ss[impl testing.internal-behavior-direct]
    // ss[impl testing.pipeline-worker-allowlist]
    // ss[impl testing.deterministic-no-sleep]
    async fn pipeline_worker_internal<A: SteadyActor>(
        mut actor: A,
        heartbeat: SteadyRx<u64>,
        generator: SteadyRx<u64>,
        logger: SteadyTx<u64>,
    ) -> Result<(), Box<dyn Error>> {
        let mut heartbeat = heartbeat.lock().await;
        let mut generator = generator.lock().await;
        let mut logger = logger.lock().await;

        while actor.is_running(
            || heartbeat.is_closed_and_empty()
                && generator.is_closed_and_empty()
                && logger.mark_closed(),
        ) {
            let clean = await_for_all!(
                actor.wait_avail(&mut heartbeat, 1),
                actor.wait_avail(&mut generator, 1),
                actor.wait_vacant(&mut logger, 1)
            );

            if actor.try_take(&mut heartbeat).is_some() || !clean {
                if let Some(&value) = actor.try_peek(&mut generator) {
                    match actor.try_send(&mut logger, value) {
                        SendOutcome::Success => {
                            actor.try_take(&mut generator).expect("internal error");
                        }
                        SendOutcome::Blocked(_) => continue,
                        SendOutcome::Timeout(_) | SendOutcome::Closed(_) => continue,
                    }
                }
            }
        }
        Ok(())
    }

    // ss[related testing.graph-for-testing]
    async fn pipeline_worker_run(
        actor: SteadyActorShadow,
        heartbeat_rx: SteadyRx<u64>,
        generator_rx: SteadyRx<u64>,
        logger_tx: SteadyTx<u64>,
    ) -> Result<(), Box<dyn Error>> {
        let actor = actor.into_spotlight([&heartbeat_rx, &generator_rx], [&logger_tx]);
        if actor.use_internal_behavior {
            pipeline_worker_internal(actor, heartbeat_rx, generator_rx, logger_tx).await
        } else {
            actor
                .simulated_behavior(sim_runners!(
                    heartbeat_rx,
                    generator_rx,
                    logger_tx
                ))
                .await
        }
    }

    // ss[related testing.graph-for-testing]
    async fn sim_tx_producer_edge(
        actor: SteadyActorShadow,
        tx: SteadyTx<u64>,
    ) -> Result<(), Box<dyn Error>> {
        let actor = actor.into_spotlight([], [&tx]);
        if actor.use_internal_behavior {
            Ok(())
        } else {
            actor.simulated_behavior(sim_runners!(tx)).await
        }
    }

    // ss[related testing.graph-for-testing]
    async fn one_u64_consumer_internal<A: SteadyActor>(
        mut actor: A,
        rx: SteadyRx<u64>,
    ) -> Result<(), Box<dyn Error>> {
        let mut rx = rx.lock().await;
        while actor.is_running(|| rx.is_closed_and_empty()) {
            let _clean = await_for_all!(actor.wait_avail(&mut rx, 1));
            let _ = actor.try_take(&mut rx);
        }
        Ok(())
    }

    // ss[verify testing.stage-manager-integration]
    // ss[verify actor.run-dispatcher]
    // ss[verify actor.shadow-spotlight]
    // ss[verify testing.internal-behavior-direct]
    #[test]
    // ss[related testing.graph-for-testing]
    fn staged_single_sim_producer_and_real_consumer_shuts_down_cleanly() -> Result<(), Box<dyn Error>> {
        SteadyRunner::test_build().run((), |mut graph| {
            let (prod_tx, prod_rx) = graph.channel_builder().with_capacity(8).build::<u64>();

            graph.actor_builder().with_name("PRODUCER").build(
                move |ctx| sim_tx_producer_edge(ctx, prod_tx.clone()),
                SoloAct,
            );
            graph.actor_builder().with_name("CONSUMER").build(
                move |ctx| {
                    let rx = prod_rx.clone();
                    async move {
                        let actor = ctx.into_spotlight([&rx], []);
                        one_u64_consumer_internal(actor, rx).await
                    }
                },
                SoloAct,
            );

            graph.start();
            let sm = graph.stage_manager();
            sm.actor_perform("PRODUCER", StageDirection::Echo(42_u64))?;
            sm.final_bow();

            graph.request_shutdown();
            graph.block_until_stopped(Duration::from_secs(5))
        })
    }

    // ss[verify testing.stage-manager-integration]
    // ss[verify testing.pipeline-worker-allowlist]
    // ss[verify philosophy.structural-hierarchy]
    // ss[verify actor.internal-behavior-logic]
    #[test]
    // ss[related testing.graph-for-testing]
    fn staged_pipeline_four_actor_graph_regression() -> Result<(), Box<dyn Error>> {
        // ss[related testing.graph-for-testing]
        const NAME_GENERATOR: &str = "GENERATOR";
        // ss[related testing.graph-for-testing]
        const NAME_HEARTBEAT: &str = "HEARTBEAT";
        // ss[related testing.graph-for-testing]
        const NAME_WORKER: &str = "WORKER";
        // ss[related testing.graph-for-testing]
        const NAME_LOGGER: &str = "LOGGER";

        SteadyRunner::test_build().run((), |mut graph| {
            let (gen_lazy, gen_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
            let (hb_lazy, hb_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
            let (log_lazy, log_rx) = graph.channel_builder().with_capacity(16).build::<u64>();

            graph
                .actor_builder()
                .with_name(NAME_GENERATOR)
                .build(move |ctx| pipeline_generator_edge(ctx, gen_lazy.clone()), SoloAct);
            graph
                .actor_builder()
                .with_name(NAME_HEARTBEAT)
                .build(move |ctx| pipeline_heartbeat_edge(ctx, hb_lazy.clone()), SoloAct);
            graph.actor_builder().with_name(NAME_WORKER).build(
                move |ctx| pipeline_worker_run(ctx, hb_rx.clone(), gen_rx.clone(), log_lazy.clone()),
                SoloAct,
            );
            graph
                .actor_builder()
                .with_name(NAME_LOGGER)
                .build(move |ctx| pipeline_logger_edge(ctx, log_rx.clone()), SoloAct);

            graph.start();

            let sm = graph.stage_manager();
            sm.actor_perform(NAME_GENERATOR, StageDirection::Echo(15_u64))?;
            sm.actor_perform(NAME_HEARTBEAT, StageDirection::Echo(100_u64))?;
            sm.actor_perform(
                NAME_LOGGER,
                StageWaitFor::Message(15_u64, Duration::from_secs(2)),
            )?;
            sm.final_bow();

            graph.request_shutdown();
            graph.block_until_stopped(Duration::from_secs(5))
        })
    }

    // #[test]
    // #[ignore] //this complex test still hangs
    // fn test_wait_available_units_shutdown() -> Result<(), Box<dyn Error>> {
    //     let mut manager = StageManager::default();
    //     let (shutdown_tx, shutdown_rx) = oneshot::channel();
    //     manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
    //     let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
    //     let mut responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
    //
    //     core_exec::spawn_detached(async move {
    //         let _ = Delay::new(Duration::from_millis(10)).await;
    //         drop(shutdown_tx); // Trigger shutdown
    //     });
    //
    //     let result = core_exec::block_on(responder.wait_available_units(5));
    //     assert!(!result);
    //     Ok(())
    // }

    #[test]
    // ss[verify testing.graph-for-testing]
    fn test_respond_with_error_path() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("test", None), 1, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        
        // Fill the response channel from the driver side to force an error in respond_with
        let backplane = manager.backplane.get(&ActorName::new("test", None)).unwrap().clone();
        core_exec::block_on(async {
            let mut guard = backplane.lock().await;
            let (tx, _) = guard.deref_mut();
            tx.push(Box::new("request")).await.unwrap();
        });

        let mut actor = DummyActor { has_data: true };
        // This test exercises the "Ok(true)" branch when empty, and "Ok(false)" when logic returns None.
        let res = responder.respond_with(|_, _| None, &mut actor)?;
        assert!(!res);
        Ok(())
    }

    // ss[related testing.graph-for-testing]
    use proptest::prelude::*;

    // ss[related testing.graph-for-testing]
    fn build_shutdown_proptest_pipeline(
        graph: &mut Graph,
    ) -> (
        LazySteadyTx<u64>,
        LazySteadyTx<u64>,
        SteadyRx<u64>,
    ) {
        // ss[related testing.graph-for-testing]
        const NAME_GENERATOR: &str = "GENERATOR";
        // ss[related testing.graph-for-testing]
        const NAME_HEARTBEAT: &str = "HEARTBEAT";
        // ss[related testing.graph-for-testing]
        const NAME_WORKER: &str = "WORKER";
        // ss[related testing.graph-for-testing]
        const NAME_LOGGER: &str = "LOGGER";

        let (gen_lazy, generator_rx) = graph.channel_builder().with_capacity(64).build::<u64>();
        let (hb_lazy, hb_rx) = graph.channel_builder().with_capacity(64).build::<u64>();
        let (log_lazy, log_rx_lazy) = graph.channel_builder().with_capacity(64).build::<u64>();
        let log_rx_out = log_rx_lazy.clone();

        let actor_builder = graph.actor_builder();

        actor_builder
            .with_name(NAME_GENERATOR)
            .never_simulate(true)
            .build(
                |ctx| async move {
                    let mut actor = ctx.into_spotlight([], []);
                    while actor.is_running(|| true) {}
                    Ok(())
                },
                SoloAct,
            );

        actor_builder
            .with_name(NAME_HEARTBEAT)
            .never_simulate(true)
            .build(
                |ctx| async move {
                    let mut actor = ctx.into_spotlight([], []);
                    while actor.is_running(|| true) {}
                    Ok(())
                },
                SoloAct,
            );

        graph.actor_builder().with_name(NAME_WORKER).build(
            move |ctx| {
                let hb = hb_rx.clone();
                let generator = generator_rx.clone();
                let log = log_lazy.clone();
                async move {
                    let actor = ctx.into_spotlight([&hb, &generator], [&log]);
                    pipeline_worker_internal(actor, hb, generator, log).await
                }
            },
            SoloAct,
        );

        actor_builder
            .with_name(NAME_LOGGER)
            .never_simulate(true)
            .build(
                |ctx| async move {
                    let mut actor = ctx.into_spotlight([], []);
                    while actor.is_running(|| true) {}
                    Ok(())
                },
                SoloAct,
            );

        (gen_lazy, hb_lazy, log_rx_out)
    }

    // ss[related testing.graph-for-testing]
    fn build_staged_puppet_pipeline(graph: &mut Graph) {
        // ss[related testing.graph-for-testing]
        const NAME_GENERATOR: &str = "GENERATOR";
        // ss[related testing.graph-for-testing]
        const NAME_HEARTBEAT: &str = "HEARTBEAT";
        // ss[related testing.graph-for-testing]
        const NAME_WORKER: &str = "WORKER";
        // ss[related testing.graph-for-testing]
        const NAME_LOGGER: &str = "LOGGER";

        let (gen_lazy, gen_rx) = graph.channel_builder().with_capacity(32).build::<u64>();
        let (hb_lazy, hb_rx) = graph.channel_builder().with_capacity(32).build::<u64>();
        let (log_lazy, log_rx) = graph.channel_builder().with_capacity(32).build::<u64>();

        graph
            .actor_builder()
            .with_name(NAME_GENERATOR)
            .build(move |ctx| pipeline_generator_edge(ctx, gen_lazy.clone()), SoloAct);
        graph
            .actor_builder()
            .with_name(NAME_HEARTBEAT)
            .build(move |ctx| pipeline_heartbeat_edge(ctx, hb_lazy.clone()), SoloAct);
        graph.actor_builder().with_name(NAME_WORKER).build(
            move |ctx| pipeline_worker_run(ctx, hb_rx.clone(), gen_rx.clone(), log_lazy.clone()),
            SoloAct,
        );
        graph
            .actor_builder()
            .with_name(NAME_LOGGER)
            .build(move |ctx| pipeline_logger_edge(ctx, log_rx.clone()), SoloAct);
    }

    // ss[related testing.graph-for-testing]
    fn setup_side_channel_responder(capacity: usize) -> (StageManager, SideChannelResponder) {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("EDGE", None), capacity, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        (manager, responder)
    }

    /// Voting-phase bound for graph integration properties (work must finish before shutdown).
    // ss[related testing.graph-for-testing]
    fn integration_vote_timeout() -> Duration {
        Duration::from_millis(500)
    }

    /// Poll logger availability until `pred` holds or the deadline elapses.
    // ss[related testing.graph-for-testing]
    fn poll_log_avail<F>(log_rx: &SteadyRx<u64>, deadline: Duration, mut pred: F)
    where
        F: FnMut(usize) -> bool,
    {
        let end = Instant::now() + deadline;
        loop {
            let avail = {
                let mut rx = core_exec::block_on(log_rx.lock());
                rx.avail_units()
            };
            if pred(avail) || Instant::now() >= end {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Request shutdown and bound only the cooperative voting/drain phase.
    // ss[related testing.graph-for-testing]
    fn shutdown_started_graph(mut graph: Graph) -> Result<(), Box<dyn Error>> {
        graph.request_shutdown();
        graph.block_until_stopped(integration_vote_timeout())
    }

    /// Heavy graph integration properties: low case count (each case spawns OS-thread actors).
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 32,
            .. ProptestConfig::default()
        })]

        /// Property: random injected traffic + early shutdown still stops cleanly (puppet graph, no `run()` on worker).
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify graph.block-until-stopped]
        // ss[verify verify.process.proptest]
        fn proptest_pipeline_random_messages_clean_shutdown(
            gen_values in prop::collection::vec(0u64..10_000, 0..32),
            early_shutdown in any::<bool>(),
        ) {
            prop_assume!(early_shutdown || !gen_values.is_empty());

            let hb_beats = gen_values.len();
            let gen_values_clone = gen_values.clone();
            let logged = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
            let logged_cap = logged.clone();
            SteadyRunner::test_build()
                .run((), move |mut graph| {
                    let (gen_tx, hb_tx, log_rx) = build_shutdown_proptest_pipeline(&mut graph);
                    gen_tx.testing_send_all(gen_values_clone.clone(), true);
                    if hb_beats > 0 {
                        hb_tx.testing_send_all(vec![0u64; hb_beats], true);
                    } else {
                        // Worker veto requires closed inputs; empty traffic still must close hb.
                        hb_tx.testing_close();
                    }
                    graph.start();
                    if !early_shutdown && !gen_values_clone.is_empty() {
                        poll_log_avail(
                            &log_rx,
                            Duration::from_millis(200),
                            |n| n >= gen_values_clone.len(),
                        );
                    }
                    shutdown_started_graph(graph)?;
                    let mut rx = core_exec::block_on(log_rx.lock());
                    let mut out = Vec::new();
                    while let Some(v) = rx.try_take() {
                        out.push(v);
                    }
                    *logged_cap.lock().expect("logged lock") = out;
                    Ok(())
                })
                .expect("runner should complete");
            let logged = logged.lock().expect("logged lock").clone();

            if early_shutdown {
                prop_assert!(logged.len() <= gen_values.len());
            } else {
                prop_assert_eq!(logged.len(), gen_values.len());
                prop_assert_eq!(logged, gen_values);
            }
        }

        /// Property: an empty testing graph reaches Running via `start_with_timeout`.
        #[test]
        // ss[verify graph.for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_empty_graph_start_with_timeout_reaches_running(
            timeout_ms in 50u64..2_000,
        ) {
            let mut graph = GraphBuilder::for_testing().build(());
            let timeout = Duration::from_millis(timeout_ms);
            prop_assert!(graph.start_with_timeout(timeout));
            prop_assert!(graph
                .runtime_state
                .read()
                .is_in_state(&[GraphLivelinessState::Running]));
            shutdown_started_graph(graph).expect("graph should shut down");
        }

        /// Property: staged puppet graph echoes generator traffic and logger receives it (never `run()` on worker).
        #[test]
        // ss[verify testing.stage-manager-integration]
        // ss[verify testing.pipeline-worker-allowlist]
        // ss[verify verify.process.proptest]
        fn proptest_staged_puppet_echo_wait_matrix(
            gen_value in 0u64..10_000,
            hb_value in 0u64..10_000,
            use_echo_at in any::<bool>(),
            use_message_at in any::<bool>(),
            timeout_ms in 50u64..200,
        ) {
            SteadyRunner::test_build().run((), move |mut graph| {
                build_staged_puppet_pipeline(&mut graph);
                graph.start();
                let sm = graph.stage_manager();
                let perform_result: Result<(), Box<dyn Error>> = (|| {
                    if use_echo_at {
                        sm.actor_perform("GENERATOR", StageDirection::EchoAt(0, gen_value))?;
                    } else {
                        sm.actor_perform("GENERATOR", StageDirection::Echo(gen_value))?;
                    }
                    sm.actor_perform("HEARTBEAT", StageDirection::Echo(hb_value))?;
                    if use_message_at {
                        sm.actor_perform(
                            "LOGGER",
                            StageWaitFor::MessageAt(0, gen_value, Duration::from_millis(timeout_ms)),
                        )?;
                    } else {
                        sm.actor_perform(
                            "LOGGER",
                            StageWaitFor::Message(gen_value, Duration::from_millis(timeout_ms)),
                        )?;
                    }
                    Ok(())
                })();
                sm.final_bow();
                shutdown_started_graph(graph)?;
                perform_result
            })
            .expect("staged puppet graph should shut down cleanly");
        }

        /// Property: puppet shutdown matrix — inject traffic or stage echo, with optional early shutdown.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify graph.block-until-stopped]
        // ss[verify verify.process.proptest]
        fn proptest_puppet_shutdown_traffic_matrix(
            gen_values in prop::collection::vec(0u64..1_000, 0..24),
            use_staged_echo in any::<bool>(),
            early_shutdown in any::<bool>(),
        ) {
            prop_assume!(use_staged_echo || !gen_values.is_empty());
            let gen_values_clone = gen_values.clone();
            let expected_len = gen_values.len();
            let logged = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
            let logged_cap = logged.clone();
            SteadyRunner::test_build().run((), move |mut graph| {
                if use_staged_echo {
                    build_staged_puppet_pipeline(&mut graph);
                    graph.start();
                    let sm = graph.stage_manager();
                    let perform_result: Result<(), Box<dyn Error>> = (|| {
                        for &v in &gen_values_clone {
                            sm.actor_perform("GENERATOR", StageDirection::Echo(v))?;
                            sm.actor_perform("HEARTBEAT", StageDirection::Echo(0_u64))?;
                        }
                        Ok(())
                    })();
                    sm.final_bow();
                    shutdown_started_graph(graph)?;
                    perform_result
                } else {
                    let (gen_tx, hb_tx, log_rx) = build_shutdown_proptest_pipeline(&mut graph);
                    if !gen_values_clone.is_empty() {
                        gen_tx.testing_send_all(gen_values_clone.clone(), true);
                        hb_tx.testing_send_all(vec![0u64; gen_values_clone.len()], true);
                    }
                    graph.start();
                    if !early_shutdown && !gen_values_clone.is_empty() {
                        poll_log_avail(
                            &log_rx,
                            Duration::from_millis(200),
                            |n| n >= gen_values_clone.len(),
                        );
                    }
                    shutdown_started_graph(graph)?;
                    let mut rx = core_exec::block_on(log_rx.lock());
                    let mut out = Vec::new();
                    while let Some(v) = rx.try_take() {
                        out.push(v);
                    }
                    *logged_cap.lock().expect("logged lock") = out;
                    Ok(())
                }
            })
            .expect("shutdown matrix should complete");
            if !use_staged_echo {
                if early_shutdown {
                    let logged = logged.lock().expect("logged lock").clone();
                    prop_assert!(logged.len() <= expected_len);
                } else if expected_len > 0 {
                    let logged = logged.lock().expect("logged lock").clone();
                    prop_assert_eq!(logged.len(), expected_len);
                    prop_assert_eq!(logged, gen_values);
                }
            }
        }

        /// Property: `actor_perform_with_suffix` routes to the suffixed actor registration.
        #[test]
        // ss[verify testing.stage-manager-integration]
        // ss[verify verify.process.proptest]
        fn proptest_actor_perform_with_suffix_registers(
            suffix in 1usize..8,
            echo_value in 0u64..1_000,
        ) {
            SteadyRunner::test_build().run((), move |mut graph| {
                let (tx_lazy, _rx) = graph.channel_builder().with_capacity(8).build::<u64>();
                graph
                    .actor_builder()
                    .with_name_and_suffix("SUFFIX_ACTOR", suffix)
                    .build(move |ctx| pipeline_generator_edge(ctx, tx_lazy.clone()), SoloAct);
                graph.start();
                let sm = graph.stage_manager();
                let perform_result = sm
                    .actor_perform_with_suffix(
                        "SUFFIX_ACTOR",
                        suffix,
                        StageDirection::Echo(echo_value),
                    )
                    .map(|_| ());
                sm.final_bow();
                shutdown_started_graph(graph)?;
                perform_result
            })
            .expect("suffix actor should respond");
        }

        /// Property: puppet graph with multiple staged echoes drains without hang.
        #[test]
        // ss[verify testing.pipeline-worker-allowlist]
        // ss[verify testing.stage-manager-integration]
        // ss[verify verify.process.proptest]
        fn proptest_staged_puppet_multi_echo_sequence(
            values in prop::collection::vec(0u64..500, 1..4),
            timeout_ms in 50u64..100,
        ) {
            SteadyRunner::test_build().run((), move |mut graph| {
                build_staged_puppet_pipeline(&mut graph);
                graph.start();
                let sm = graph.stage_manager();
                let perform_result: Result<(), Box<dyn Error>> = (|| {
                    for &v in &values {
                        sm.actor_perform("GENERATOR", StageDirection::Echo(v))?;
                        sm.actor_perform("HEARTBEAT", StageDirection::Echo(0_u64))?;
                        sm.actor_perform(
                            "LOGGER",
                            StageWaitFor::Message(v, Duration::from_millis(timeout_ms)),
                        )?;
                    }
                    Ok(())
                })();
                sm.final_bow();
                shutdown_started_graph(graph)?;
                perform_result
            })
            .expect("multi-echo puppet graph should shut down");
        }

        /// Property: staged puppet with suffixed worker lane still shuts down cleanly.
        #[test]
        // ss[verify testing.pipeline-worker-allowlist]
        // ss[verify testing.stage-manager-integration]
        // ss[verify verify.process.proptest]
        fn proptest_staged_puppet_with_suffix_actor(
            gen_value in 0u64..2_000,
            suffix in 1usize..4,
        ) {
            SteadyRunner::test_build().run((), move |mut graph| {
                let (gen_lazy, gen_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
                let (hb_lazy, hb_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
                let (log_lazy, log_rx) = graph.channel_builder().with_capacity(16).build::<u64>();
                graph
                    .actor_builder()
                    .with_name("GENERATOR")
                    .build(move |ctx| pipeline_generator_edge(ctx, gen_lazy.clone()), SoloAct);
                graph
                    .actor_builder()
                    .with_name_and_suffix("HEARTBEAT", suffix)
                    .build(move |ctx| pipeline_heartbeat_edge(ctx, hb_lazy.clone()), SoloAct);
                graph.actor_builder().with_name("WORKER").build(
                    move |ctx| pipeline_worker_run(ctx, hb_rx.clone(), gen_rx.clone(), log_lazy.clone()),
                    SoloAct,
                );
                graph
                    .actor_builder()
                    .with_name("LOGGER")
                    .build(move |ctx| pipeline_logger_edge(ctx, log_rx.clone()), SoloAct);
                graph.start();
                let sm = graph.stage_manager();
                let perform_result: Result<(), Box<dyn Error>> = (|| {
                    sm.actor_perform("GENERATOR", StageDirection::Echo(gen_value))?;
                    sm.actor_perform_with_suffix("HEARTBEAT", suffix, StageDirection::Echo(0_u64))?;
                    sm.actor_perform(
                        "LOGGER",
                        StageWaitFor::Message(gen_value, Duration::from_millis(200)),
                    )?;
                    Ok(())
                })();
                sm.final_bow();
                shutdown_started_graph(graph)?;
                perform_result
            })
            .expect("suffix puppet graph should shut down");
        }
    }

    ss_proptest! {

        /// Property: `simulate_direction` echoes arbitrary values through a locked TX.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_simulate_direction_echo_roundtrip(
            cap in 2usize..16,
            value in 0u64..10_000,
            lane in 0usize..2,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
            let tx = tx_lazy.clone();
            let mut tx = core_exec::block_on(tx.lock());
            let mut actor = DummyActor { has_data: true };
            let backplane = _manager.backplane.get(&ActorName::new("EDGE", None)).unwrap().clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(StageDirection::EchoAt(lane, value)))
                    .await
                    .expect("push echo-at");
            });
            let result = responder
                .simulate_direction(&mut tx, &mut actor, lane)
                .expect("simulate direction");
            prop_assert_eq!(result, SimStepResult::DidWork);
            let rx = rx_lazy.clone();
            let mut rx = core_exec::block_on(rx.lock());
            prop_assert_eq!(rx.try_take(), Some(value));
        }

        /// Property: `simulate_wait_for` succeeds when the expected message is available.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_simulate_wait_for_message_match(
            cap in 2usize..16,
            expected in 0u64..10_000,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
            tx_lazy.testing_send_all(vec![expected], false);
            let rx = rx_lazy.clone();
            let mut rx = core_exec::block_on(rx.lock());
            let mut actor = DummyActor { has_data: true };
            let backplane = _manager.backplane.get(&ActorName::new("EDGE", None)).unwrap().clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(StageWaitFor::Message(
                        expected,
                        Duration::from_millis(500),
                    )))
                    .await
                    .expect("push wait-for");
            });
            let result = responder
                .simulate_wait_for(&mut rx, &mut actor, 0)
                .expect("simulate wait-for");
            prop_assert_eq!(result, SimStepResult::DidWork);
        }

        /// Property: `call_actor_internal` returns error for unknown actor names.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_call_actor_internal_missing_actor(_seed in 0u64..1_000) {
            let manager = StageManager::default();
            let result = manager.call_actor_internal(
                Box::new("req"),
                ActorName::new("MISSING", None),
            );
            prop_assert!(result.is_err());
        }

        /// Property: `call_actor_internal` returns OK when actor responds with OK_MESSAGE.
        #[test]
        // ss[verify testing.stage-manager-integration]
        // ss[verify verify.process.proptest]
        fn proptest_call_actor_internal_ok_response(
            cap in 2usize..16,
            _seed in 0u64..100,
        ) {
            let mut manager = StageManager::default();
            let (_shutdown_tx, shutdown_rx) = oneshot::channel();
            let name = ActorName::new("OK_ACTOR", None);
            manager.register_node(name, cap, shutdown_rx);
            let node_side = manager.node_tx_rx(name).unwrap();
            core_exec::spawn_detached(async move {
                let mut guard = node_side.lock().await;
                let ((tx_resp, rx_req), _) = guard.deref_mut();
                if rx_req.pop().await.is_some() {
                    let _ = tx_resp.push(Box::new(OK_MESSAGE.to_string())).await;
                }
            });
            let result = manager.call_actor_internal(Box::new("ping"), name);
            prop_assert!(result.is_ok());
        }

        /// Property: `simulate_direction` returns NoWork when EchoAt lane index mismatches.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_simulate_direction_echo_at_lane_mismatch(
            cap in 2usize..16,
            value in 0u64..1_000,
            lane in 0usize..2,
        ) {
            let wrong_lane = 1 - lane;
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, _rx_lazy) = builder.build_channel::<u64>();
            let tx_steady = tx_lazy.clone();
            let mut tx = core_exec::block_on(tx_steady.lock());
            let mut actor = DummyActor { has_data: true };
            let backplane = _manager.backplane.get(&ActorName::new("EDGE", None)).unwrap().clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(StageDirection::EchoAt(lane, value)))
                    .await
                    .expect("push echo-at");
            });
            let result = responder
                .simulate_direction(&mut tx, &mut actor, wrong_lane)
                .expect("simulate direction");
            prop_assert_eq!(result, SimStepResult::NoWork);
        }

        /// Property: `simulate_wait_for` reports mismatch when message differs from expected.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_simulate_wait_for_message_mismatch(
            cap in 2usize..16,
            expected in 0u64..500,
            actual in 500u64..1_000,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
            tx_lazy.testing_send_all(vec![actual], false);
            let rx = rx_lazy.clone();
            let mut rx = core_exec::block_on(rx.lock());
            let mut actor = DummyActor { has_data: true };
            let backplane = _manager.backplane.get(&ActorName::new("EDGE", None)).unwrap().clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(StageWaitFor::Message(
                        expected,
                        Duration::from_millis(500),
                    )))
                    .await
                    .expect("push wait-for");
            });
            let result = responder
                .simulate_wait_for(&mut rx, &mut actor, 0)
                .expect("simulate wait-for");
            // Mismatch produces a failure response (DidWork) rather than OK_MESSAGE.
            prop_assert_eq!(result, SimStepResult::DidWork);
        }

        /// Property: `should_apply` returns None when side channel queue is empty.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_should_apply_empty_queue(cap in 2usize..16) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let result = core_exec::block_on(responder.should_apply::<i32>());
            prop_assert_eq!(result, None);
        }

        /// Property: `respond_with` returns false when side channel has no messages.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_respond_with_empty_channel(cap in 2usize..16) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let mut actor = DummyActor { has_data: false };
            let result = responder.respond_with(|_, _| Some(Box::new(OK_MESSAGE)), &mut actor);
            prop_assert!(matches!(result, Ok(false)));
        }

        /// Property: `echo_responder_bundle` fans out a staged message to every TX lane.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_echo_responder_bundle_roundtrip(
            cap in 2usize..16,
            value in -1_000i32..1_000,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx0_lazy, rx0_lazy) = builder.build_channel::<i32>();
            let (tx1_lazy, rx1_lazy) = builder.build_channel::<i32>();
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(value))
                    .await
                    .expect("push echo payload");
            });
            let mut actor = DummyActor { has_data: true };
            let tx0 = tx0_lazy.clone();
            let tx1 = tx1_lazy.clone();
            let ok = core_exec::block_on(async {
                let mut bundle = TxBundle::new();
                bundle.push(tx0.try_lock().expect("tx0"));
                bundle.push(tx1.try_lock().expect("tx1"));
                responder.echo_responder_bundle(&mut actor, &mut bundle).await
            });
            prop_assert!(ok.expect("bundle echo"));
            prop_assert_eq!(rx0_lazy.testing_take_all(), vec![value]);
            prop_assert_eq!(rx1_lazy.testing_take_all(), vec![value]);
        }

        /// Property: `equals_responder_bundle` succeeds when every RX lane matches the staged value.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_equals_responder_bundle_match(
            cap in 2usize..16,
            value in -500i32..500,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx0_lazy, rx0_lazy) = builder.build_channel::<i32>();
            let (tx1_lazy, rx1_lazy) = builder.build_channel::<i32>();
            tx0_lazy.testing_send_all(vec![value], false);
            tx1_lazy.testing_send_all(vec![value], false);
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(value))
                    .await
                    .expect("push equals payload");
            });
            let mut actor = DummyActor { has_data: true };
            let rx0 = rx0_lazy.clone();
            let rx1 = rx1_lazy.clone();
            let ok = core_exec::block_on(async {
                let mut bundle = RxBundle::new();
                bundle.push(rx0.try_lock().expect("rx0"));
                bundle.push(rx1.try_lock().expect("rx1"));
                responder.equals_responder_bundle(&mut actor, &mut bundle).await
            });
            prop_assert!(ok.expect("bundle equals match"));
        }

        /// Property: `equals_responder_bundle` fails when any RX lane differs from the staged value.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_equals_responder_bundle_mismatch(
            cap in 2usize..16,
            expected in -200i32..200,
            other in 201i32..400,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx0_lazy, rx0_lazy) = builder.build_channel::<i32>();
            let (tx1_lazy, rx1_lazy) = builder.build_channel::<i32>();
            tx0_lazy.testing_send_all(vec![expected], false);
            tx1_lazy.testing_send_all(vec![other], false);
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(expected))
                    .await
                    .expect("push equals payload");
            });
            let mut actor = DummyActor { has_data: true };
            let rx0 = rx0_lazy.clone();
            let rx1 = rx1_lazy.clone();
            let ok = core_exec::block_on(async {
                let mut bundle = RxBundle::new();
                bundle.push(rx0.try_lock().expect("rx0"));
                bundle.push(rx1.try_lock().expect("rx1"));
                responder.equals_responder_bundle(&mut actor, &mut bundle).await
            });
            prop_assert!(ok.expect("bundle equals mismatch"));
        }

        /// Property: `wait_avail` unblocks once a staged message arrives on the backplane.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_wait_avail_unblocks_on_message(cap in 2usize..16) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::spawn_detached(async move {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                let _ = bp_tx.push(Box::new(7i32)).await;
            });
            core_exec::block_on(responder.wait_avail());
            prop_assert!(responder.avail() >= 1);
        }

        /// Property: `wait_available_units` returns true when enough requests are queued.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_wait_available_units_ready(
            cap in 2usize..16,
            need in 1usize..4,
        ) {
            prop_assume!(need <= cap);
            let mut manager = StageManager::default();
            let (_shutdown_tx, shutdown_rx) = oneshot::channel();
            manager.register_node(ActorName::new("EDGE", None), cap, shutdown_rx);
            let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
            let mut responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
            let backplane = manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                for _ in 0..need {
                    bp_tx.push(Box::new(1i32)).await.expect("push request");
                }
            });
            let ready = core_exec::block_on(responder.wait_available_units(need));
            prop_assert!(ready);
        }

        /// Property: `simulate_direction` surfaces `SendOutcome::Blocked` on a full channel.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_simulate_direction_blocked_on_full_channel(
            value in 0u64..1_000,
        ) {
            let cap = 1usize;
            let (_manager, responder) = setup_side_channel_responder(4);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, _rx_lazy) = builder.build_channel::<u64>();
            let tx_steady = tx_lazy.clone();
            let mut tx = core_exec::block_on(tx_steady.lock());
            let _ = tx.shared_try_send(99_u64);
            let mut actor = DummyActor { has_data: true };
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(StageDirection::Echo(value)))
                    .await
                    .expect("push echo");
            });
            let result = responder
                .simulate_direction(&mut tx, &mut actor, 0)
                .expect("simulate direction");
            prop_assert_eq!(result, SimStepResult::DidWork);
        }

        /// Property: `simulate_wait_for` with `MessageAt` succeeds on the matching lane.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_simulate_wait_for_message_at_lane(
            cap in 2usize..16,
            expected in 0u64..5_000,
            lane in 0usize..2,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
            tx_lazy.testing_send_all(vec![expected], false);
            let rx = rx_lazy.clone();
            let mut rx = core_exec::block_on(rx.lock());
            let mut actor = DummyActor { has_data: true };
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(StageWaitFor::MessageAt(
                        lane,
                        expected,
                        Duration::from_millis(500),
                    )))
                    .await
                    .expect("push wait-for-at");
            });
            let result = responder
                .simulate_wait_for(&mut rx, &mut actor, lane)
                .expect("simulate wait-for");
            prop_assert_eq!(result, SimStepResult::DidWork);
        }

        /// Property: `simulate_wait_for` ignores `MessageAt` when the lane index mismatches.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_simulate_wait_for_message_at_lane_mismatch(
            cap in 2usize..16,
            expected in 0u64..500,
            lane in 0usize..2,
        ) {
            let wrong_lane = 1 - lane;
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, rx_lazy) = builder.build_channel::<u64>();
            tx_lazy.testing_send_all(vec![expected], false);
            let rx = rx_lazy.clone();
            let mut rx = core_exec::block_on(rx.lock());
            let mut actor = DummyActor { has_data: true };
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(StageWaitFor::MessageAt(
                        lane,
                        expected,
                        Duration::from_millis(500),
                    )))
                    .await
                    .expect("push wait-for-at");
            });
            let result = responder
                .simulate_wait_for(&mut rx, &mut actor, wrong_lane)
                .expect("simulate wait-for");
            prop_assert_eq!(result, SimStepResult::NoWork);
        }

        /// Property: `simulate_wait_for` times out when the expected message never arrives.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_simulate_wait_for_timeout(cap in 2usize..16, expected in 0u64..500) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (_tx_lazy, rx_lazy) = builder.build_channel::<u64>();
            let rx = rx_lazy.clone();
            let mut rx = core_exec::block_on(rx.lock());
            let mut actor = DummyActor { has_data: true };
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(StageWaitFor::Message(
                        expected,
                        Duration::from_millis(0),
                    )))
                    .await
                    .expect("push wait-for");
            });
            let first = responder
                .simulate_wait_for(&mut rx, &mut actor, 0)
                .expect("first wait-for");
            prop_assert_eq!(first, SimStepResult::NoWork);
            let second = responder
                .simulate_wait_for(&mut rx, &mut actor, 0)
                .expect("timeout wait-for");
            prop_assert_eq!(second, SimStepResult::DidWork);
        }

        /// Property: `call_actor_internal` maps actor TIMEOUT responses to errors.
        #[test]
        // ss[verify testing.stage-manager-integration]
        // ss[verify verify.process.proptest]
        fn proptest_call_actor_internal_timeout_response(cap in 2usize..16) {
            let mut manager = StageManager::default();
            let (_shutdown_tx, shutdown_rx) = oneshot::channel();
            let name = ActorName::new("TIMEOUT_ACTOR", None);
            manager.register_node(name, cap, shutdown_rx);
            let node_side = manager.node_tx_rx(name).unwrap();
            core_exec::spawn_detached(async move {
                let mut guard = node_side.lock().await;
                let ((tx_resp, rx_req), _) = guard.deref_mut();
                if rx_req.pop().await.is_some() {
                    let _ = tx_resp.push(Box::new(TIMEOUT.to_string())).await;
                }
            });
            let result = manager.call_actor_internal(Box::new("ping"), name);
            prop_assert!(result.is_err());
            prop_assert!(result.unwrap_err().to_string().contains(TIMEOUT));
        }

        /// Property: `wait_available_units` returns true immediately when enough requests are queued.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_wait_available_units_already_ready(
            cap in 4usize..16,
            need in 1usize..4,
        ) {
            prop_assume!(need <= cap);
            let mut manager = StageManager::default();
            let (_shutdown_tx, shutdown_rx) = oneshot::channel();
            manager.register_node(ActorName::new("EDGE", None), cap, shutdown_rx);
            let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
            let mut responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
            let backplane = manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                for _ in 0..need {
                    bp_tx.push(Box::new(1i32)).await.expect("push request");
                }
            });
            let ready = core_exec::block_on(responder.wait_available_units(need));
            prop_assert!(ready);
        }

        /// Property: `wait_available_units` returns false when the node shutdown channel terminates first.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_wait_available_units_shutdown_while_waiting(cap in 4usize..16) {
            let mut manager = StageManager::default();
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            manager.register_node(ActorName::new("EDGE", None), cap, shutdown_rx);
            let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
            let mut responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
            drop(shutdown_tx);
            let ready = core_exec::block_on(responder.wait_available_units(cap));
            prop_assert!(!ready);
        }

        /// Property: `echo_responder_bundle` declines when the staged payload type mismatches.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_echo_responder_bundle_type_mismatch(
            cap in 2usize..16,
            value in -500i32..500,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx0_lazy, rx0_lazy) = builder.build_channel::<i32>();
            let (tx1_lazy, _rx1_lazy) = builder.build_channel::<i32>();
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(value.to_string()))
                    .await
                    .expect("push wrong type");
            });
            let mut actor = DummyActor { has_data: true };
            let tx0 = tx0_lazy.clone();
            let tx1 = tx1_lazy.clone();
            let ok = core_exec::block_on(async {
                let mut bundle = TxBundle::new();
                bundle.push(tx0.try_lock().expect("tx0"));
                bundle.push(tx1.try_lock().expect("tx1"));
                responder.echo_responder_bundle(&mut actor, &mut bundle).await
            });
            prop_assert_eq!(ok.expect("bundle echo"), false);
            prop_assert!(rx0_lazy.testing_take_all().is_empty());
        }

        /// Property: `equals_responder_bundle` declines when the staged payload type mismatches.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_equals_responder_bundle_type_mismatch(
            cap in 2usize..16,
            value in -500i32..500,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
            tx_lazy.testing_send_all(vec![value], false);
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(value.to_string()))
                    .await
                    .expect("push wrong type");
            });
            let mut actor = DummyActor { has_data: true };
            let rx = rx_lazy.clone();
            let ok = core_exec::block_on(async {
                let mut bundle = RxBundle::new();
                bundle.push(rx.try_lock().expect("rx"));
                responder.equals_responder_bundle(&mut actor, &mut bundle).await
            });
            prop_assert_eq!(ok.expect("bundle equals"), false);
        }

        /// Property: `should_apply` returns Some(false) when the queued message type mismatches.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_should_apply_type_mismatch(cap in 2usize..16, value in -500i32..500) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(value.to_string()))
                    .await
                    .expect("push wrong type");
            });
            let result = core_exec::block_on(responder.should_apply::<i32>());
            prop_assert_eq!(result, Some(false));
        }

        /// Property: `respond_with` pops the request and returns true when the handler succeeds.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_respond_with_success_pops_request(
            cap in 2usize..16,
            payload in -1_000i32..1_000,
        ) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let backplane = _manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(payload))
                    .await
                    .expect("push payload");
            });
            let mut actor = DummyActor { has_data: true };
            let handled = responder.respond_with(
                |message, _actor| Some(Box::new(format!("echo:{message:?}"))),
                &mut actor,
            );
            prop_assert_eq!(handled.expect("respond_with"), true);
            prop_assert_eq!(responder.avail(), 0);
        }

        /// Property: `respond_with` errors when the actor→test response channel is full.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_respond_with_full_response_channel_errors(cap in 1usize..4) {
            let mut manager = StageManager::default();
            let (_shutdown_tx, shutdown_rx) = oneshot::channel();
            manager.register_node(ActorName::new("EDGE", None), cap, shutdown_rx);
            let node_arc = manager.node_tx_rx(ActorName::new("EDGE", None)).unwrap();
            let backplane = manager
                .backplane
                .get(&ActorName::new("EDGE", None))
                .unwrap()
                .clone();
            let responder = SideChannelResponder::new(node_arc.clone(), ActorIdentity::default());
            core_exec::block_on(async {
                let mut guard = node_arc.lock().await;
                let ((tx_resp, _), _) = guard.deref_mut();
                for _ in 0..cap {
                    tx_resp
                        .push(Box::new("fill"))
                        .await
                        .expect("fill response channel");
                }
            });
            core_exec::block_on(async {
                let mut guard = backplane.lock().await;
                let (bp_tx, _) = guard.deref_mut();
                bp_tx
                    .push(Box::new(42i32))
                    .await
                    .expect("queue request");
            });
            let mut actor = DummyActor { has_data: true };
            let err = responder.respond_with(|_, _| Some(Box::new("ok")), &mut actor);
            prop_assert!(err.is_err());
        }

        /// Property: `echo_responder_bundle` returns false when the side channel has no staged message.
        #[test]
        // ss[verify testing.graph-for-testing]
        // ss[verify verify.process.proptest]
        fn proptest_echo_responder_bundle_empty_returns_false(cap in 2usize..16) {
            let (_manager, responder) = setup_side_channel_responder(cap);
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx0_lazy, _rx0_lazy) = builder.build_channel::<i32>();
            let (tx1_lazy, _rx1_lazy) = builder.build_channel::<i32>();
            let mut actor = DummyActor { has_data: true };
            let tx0 = tx0_lazy.clone();
            let tx1 = tx1_lazy.clone();
            let ok = core_exec::block_on(async {
                let mut bundle = TxBundle::new();
                bundle.push(tx0.try_lock().expect("tx0"));
                bundle.push(tx1.try_lock().expect("tx1"));
                responder
                    .echo_responder_bundle(&mut actor, &mut bundle)
                    .await
            });
            prop_assert_eq!(ok.expect("echo bundle"), false);
        }
    }
}
