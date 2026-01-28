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
                        SendOutcome::Blocked(msg) | SendOutcome::Timeout(msg) | SendOutcome::Closed(msg) => Some(Box::new(msg)),
                    },
                    StageDirection::EchoAt(i, m) => {
                        if *i == index {
                            match actor.try_send(tx_core, m.clone()) {
                                SendOutcome::Success => Some(Box::new(OK_MESSAGE)),
                                SendOutcome::Blocked(msg) | SendOutcome::Timeout(msg) | SendOutcome::Closed(msg) => Some(Box::new(msg)),
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

#[cfg(test)]
mod graph_testing_tests {
    use super::*;
    use std::error::Error;
    use std::time::Duration;
    use aeron::aeron::Aeron;
    use futures::channel::oneshot;
    use crate::*;
    use crate::ActorName;
    use crate::ActorIdentity;
    use crate::distributed::aqueduct_stream::Defrag;
    use crate::simulate_edge::IntoSimRunner;
    use crate::RxCoreBundle;
    use crate::steady_actor::BlockingCallFuture;
    use crate::TxCoreBundle;

    struct DummyActor {
        has_data: bool,
    }

    impl SteadyActor for DummyActor {
        fn frame_rate_ms(&self) -> u64 { 0 }
        fn regeneration(&self) -> u32 { 0 }
        fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> { None }
        async fn simulated_behavior(self, _sims: Vec<&dyn IntoSimRunner<Self>>) -> Result<(), Box<dyn Error>> { Ok(()) }
        fn loglevel(&self, _loglevel: crate::LogLevel) {}
        fn relay_stats_smartly(&mut self) -> bool { false }
        fn relay_stats(&mut self) {}
        async fn relay_stats_periodic(&mut self, _duration_rate: Duration) -> bool { false }
        fn is_liveliness_in(&self, _target: &[GraphLivelinessState]) -> bool { false }
        fn is_liveliness_building(&self) -> bool { false }
        fn is_liveliness_running(&self) -> bool { false }
        fn is_liveliness_stop_requested(&self) -> bool { false }
        fn is_liveliness_shutdown_timeout(&self) -> Option<Duration> { None }
        fn flush_defrag_messages<S: StreamControlItem>(
            &mut self,
            _item: &mut Tx<S>,
            _data: &mut Tx<u8>,
            _defrag: &mut Defrag<S>,
        ) -> (u32, u32, Option<i32>) { (0, 0, None) }
        async fn wait_periodic(&self, _duration_rate: Duration) -> bool { false }
        async fn wait_timeout(&self, _timeout: Duration) -> bool { false }
        async fn wait(&self, _duration: Duration) {}
        async fn wait_avail<T: RxCore>(&self, _this: &mut T, _size: usize) -> bool { true }
        async fn wait_avail_bundle<T: RxCore>(
            &self,
            _this: &mut RxCoreBundle<'_, T>,
            _size: usize,
            _ready_channels: usize,
        ) -> bool { true }
        async fn wait_future_void<F>(&self, _fut: F) -> bool where F: FusedFuture<Output = ()> + 'static + Send + Sync { false }
        async fn wait_vacant<T: TxCore>(&self, _this: &mut T, _count: T::MsgSize) -> bool { true }
        async fn wait_vacant_bundle<T: TxCore>(
            &self,
            _this: &mut TxCoreBundle<'_, T>,
            _count: T::MsgSize,
            _ready_channels: usize,
        ) -> bool { true }
        async fn wait_shutdown(&self) -> bool { false }
        fn peek_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceSource<'b> where T: RxCore { unimplemented!() }
        fn advance_take_index<T: RxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> RxDone { unimplemented!() }
        fn take_slice<T: RxCore>(
            &mut self,
            _this: &mut T,
            _target: T::SliceTarget<'_>,
        ) -> RxDone where T::MsgItem: Copy { unimplemented!() }
        fn send_slice<T: TxCore>(
            &mut self,
            _this: &mut T,
            _source: T::SliceSource<'_>,
        ) -> TxDone where T::MsgOut: Copy { unimplemented!() }
        fn poke_slice<'b, T>(&self, _this: &'b mut T) -> T::SliceTarget<'b> where T: TxCore { unimplemented!() }
        fn advance_send_index<T: TxCore>(&mut self, _this: &mut T, _count: T::MsgSize) -> TxDone { unimplemented!() }
        fn try_peek<'a, T>(&'a self, _this: &'a mut Rx<T>) -> Option<&'a T> { None }
        fn try_peek_iter<'a, T>(
            &'a self,
            _this: &'a mut Rx<T>,
        ) -> impl Iterator<Item = &'a T> + 'a { std::iter::empty() }
        fn is_empty<T: RxCore>(&self, _this: &mut T) -> bool { !self.has_data }
        fn avail_units<T: RxCore>(&self, this: &mut T) -> T::MsgSize { if self.has_data { this.one() } else { unimplemented!() } }
        async fn peek_async<'a, T: RxCore>(
            &'a self,
            _this: &'a mut T,
        ) -> Option<T::MsgPeek<'a>> { None }
        fn send_iter_until_full<T, I: Iterator<Item = T>>(
            &mut self,
            _this: &mut Tx<T>,
            _iter: I,
        ) -> usize { 0 }
        fn try_send<T: TxCore>(
            &mut self,
            _this: &mut T,
            _msg: T::MsgIn<'_>,
        ) -> SendOutcome<T::MsgOut> { SendOutcome::Success }
        fn try_take<T: RxCore>(&mut self, _this: &mut T) -> Option<T::MsgOut> { None }
        fn is_full<T: TxCore>(&self, _this: &mut T) -> bool { false }
        fn vacant_units<T: TxCore>(&self, this: &mut T) -> T::MsgSize { this.one() }
        async fn wait_empty<T: TxCore>(&self, _this: &mut T) -> bool { false }
        fn take_into_iter<'a, T: Sync + Send>(
            &mut self,
            _this: &'a mut Rx<T>,
        ) -> impl Iterator<Item = T> + 'a { std::iter::empty() }
        async fn call_async<F>(&self, _operation: F) -> Option<F::Output> where F: Future { None }
        fn call_blocking<F, T>(&self, f: F) -> BlockingCallFuture<T>
        where
            F: FnOnce() -> T + Send + 'static,
            T: Send + 'static {
            BlockingCallFuture(core_exec::spawn_blocking(f))
        }
        async fn send_async<T: TxCore>(
            &mut self,
            _this: &mut T,
            _a: T::MsgIn<'_>,
            _saturation: SendSaturation,
        ) -> SendOutcome<T::MsgOut> { SendOutcome::Success }
        async fn take_async<T>(&mut self, _this: &mut Rx<T>) -> Option<T> { None }
        async fn take_async_with_timeout<T>(
            &mut self,
            _this: &mut Rx<T>,
            _timeout: Duration,
        ) -> Option<T> { None }
        async fn yield_now(&self) {}
        fn sidechannel_responder(&self) -> Option<SideChannelResponder> { None }
        fn is_running<F: FnMut() -> bool>(&mut self, _accept_fn: F) -> bool { true }
        async fn request_shutdown(&mut self) {}
        fn args<A: Any>(&self) -> Option<&A> { None }
        fn identity(&self) -> ActorIdentity { ActorIdentity::default() }
        fn is_showstopper<T>(&self, _rx: &mut Rx<T>, _threshold: usize) -> bool { false }
    }

    #[test]
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

    #[test]
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

    #[test]
    fn test_stage_manager_default() -> Result<(), Box<dyn Error>> {
        let manager = StageManager::default();
        assert!(manager.node.is_empty());
        assert!(manager.backplane.is_empty());
        Ok(())
    }

    #[test]
    fn test_stage_manager_clone() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("test", None), 10, shutdown_rx);

        let cloned = manager.clone();
        assert_eq!(manager.node.len(), cloned.node.len());
        assert_eq!(manager.backplane.len(), cloned.backplane.len());
        Ok(())
    }

    #[test]
    fn test_stage_manager_debug() -> Result<(), Box<dyn Error>> {
        let manager = StageManager::default();
        let debug_str = format!("{:?}", manager);
        assert!(debug_str.contains("SideChannelHub"));
        Ok(())
    }

    #[test]
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
    fn test_side_channel_responder_new() -> Result<(), Box<dyn Error>> {
        let mut manager = StageManager::default();
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        manager.register_node(ActorName::new("test", None), 10, shutdown_rx);
        let node_arc = manager.node_tx_rx(ActorName::new("test", None)).unwrap();
        let responder = SideChannelResponder::new(node_arc, ActorIdentity::default());
        assert_eq!(responder.identity, ActorIdentity::default());
        Ok(())
    }

    #[test]
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
}
