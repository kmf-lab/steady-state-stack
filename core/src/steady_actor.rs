use futures::FutureExt;
use std::future::Future;
use std::time::{Duration, Instant};
use futures_util::future::{FusedFuture};
use std::any::Any;
use std::error::Error;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use aeron::aeron::Aeron;
use futures_util::select;
use log::*;
use crate::{steady_config, ActorIdentity, GraphLivelinessState, Rx, RxCoreBundle, SendSaturation, Tx, TxCoreBundle};
use crate::graph_testing::SideChannelResponder;
use crate::monitor::{RxMetaData, TxMetaData};
use crate::monitor_telemetry::SteadyTelemetry;
use crate::steady_rx::{RxDone, RxMetaDataProvider};
use crate::steady_tx::{TxDone, TxMetaDataProvider};
use crate::telemetry::setup;
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::steady_actor_spotlight::SteadyActorSpotlight;
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::distributed::aqueduct_stream::{Defrag, StreamControlItem};
use crate::loop_driver::pin_mut;
use crate::simulate_edge::IntoSimRunner;
use futures_util::lock::Mutex;
use std::sync::atomic::AtomicBool;

impl SteadyActorShadow {
    /// Converts this actor shadow into a local monitor (spotlight) instance.
    ///
    /// # Parameters
    /// - `rx_mons`: Array of receiver monitor trait objects.
    /// - `tx_mons`: Array of transmitter monitor trait objects.
    ///
    /// # Returns
    /// A new `SteadyActorSpotlight` instance with the provided metadata.
    pub fn into_spotlight<const RX_LEN: usize, const TX_LEN: usize>(
        self,
        rx_mons: [&dyn RxMetaDataProvider; RX_LEN],
        tx_mons: [&dyn TxMetaDataProvider; TX_LEN],
    ) -> SteadyActorSpotlight<RX_LEN, TX_LEN> {
        let rx_meta = rx_mons
            .iter()
            .map(|rx| rx.meta_data())
            .collect::<Vec<_>>()
            .try_into()
            .expect("Length mismatch should never occur");

        let tx_meta = tx_mons
            .iter()
            .map(|tx| tx.meta_data())
            .collect::<Vec<_>>()
            .try_into()
            .expect("Length mismatch should never occur");

        self.into_spotlight_internal(rx_meta, tx_meta)
    }

    /// Internal conversion to a local monitor using explicit metadata arrays.
    ///
    /// # Parameters
    /// - `rx_mons`: Array of receiver metadata.
    /// - `tx_mons`: Array of transmitter metadata.
    ///
    /// # Returns
    /// A new `SteadyActorSpotlight` instance.
    pub fn into_spotlight_internal<const RX_LEN: usize, const TX_LEN: usize>(
        self,
        rx_mons: [RxMetaData; RX_LEN],
        tx_mons: [TxMetaData; TX_LEN],
    ) -> SteadyActorSpotlight<RX_LEN, TX_LEN> {
        let (send_rx, send_tx, state) = if (self.frame_rate_ms > 0)
            && (steady_config::TELEMETRY_HISTORY || steady_config::TELEMETRY_SERVER) {
            let mut rx_meta_data = Vec::new();
            let mut rx_inverse_local_idx = [0; RX_LEN];
            rx_mons.iter().enumerate().for_each(|(c, md)| {
                assert!(md.id < usize::MAX);
                trace!("register rx {}", md.id);

                rx_inverse_local_idx[c] = md.id;
                rx_meta_data.push(md.clone());
            });

            let mut tx_meta_data = Vec::new();
            let mut tx_inverse_local_idx = [0; TX_LEN];
            tx_mons.iter().enumerate().for_each(|(c, md)| {
                assert!(md.id < usize::MAX);
                trace!("register tx {}", md.id);
                tx_inverse_local_idx[c] = md.id;
                tx_meta_data.push(md.clone());
            });

            setup::construct_telemetry_channels(
                &self,
                rx_meta_data,
                rx_inverse_local_idx,
                tx_meta_data,
                tx_inverse_local_idx,
            )
        } else {
            (None, None, None)
        };

        SteadyActorSpotlight::<RX_LEN, TX_LEN> {
            telemetry: SteadyTelemetry {
                send_rx,
                send_tx,
                state,
                dirty: AtomicBool::new(false),
            },
            last_telemetry_send: Instant::now(),
            ident: self.ident,
            is_in_graph: self.is_in_graph,
            runtime_state: self.runtime_state,
            oneshot_shutdown: self.oneshot_shutdown,
            last_periodic_wait: Default::default(),
            actor_start_time: self.actor_start_time,
            node_tx_rx: self.node_tx_rx.clone(),
            frame_rate_ms: self.frame_rate_ms,
            args: self.args,
            _team_id: self.team_id,
            is_running_iteration_count: 0,
            regeneration: self.regeneration,
            aeron_meda_driver: self.aeron_meda_driver.clone(),
            use_internal_behavior: self.use_internal_behavior,
            shutdown_barrier: self.shutdown_barrier.clone(),

        }
    }
}

/// Represents the outcome of a send operation on a channel.
#[derive(Default)]
pub enum SendOutcome<X> {
    /// The send operation was successful.
    #[default]
    Success,
    /// The channel is full (returned by non-blocking try_send).
    Blocked(X),
    /// The operation timed out (likely due to telemetry deadlines).
    Timeout(X),
    /// The channel is closed or a shutdown was signaled.
    Closed(X),
}

impl<X> SendOutcome<X> {
    /// Returns `true` if the send operation succeeded.
    pub fn is_sent(&self) -> bool {
        match self {
            SendOutcome::Success => true,
            _ => false,
        }
    }

    /// Panics with the provided message if the outcome is not `Success`.
    /// Returns `true` if the outcome is `Success`.
    pub fn expect(self, msg: &'static str) -> bool {
        match self {
            SendOutcome::Success => true,
            _ => panic!("{}", msg),
        }
    }
}

/// Struct returned for the monitoring and result fetching of the long running blocking call.
pub struct BlockingCallFuture<T>(pub Pin<Box<dyn FusedFuture<Output = T> + Send>>);

impl<T> std::future::Future for BlockingCallFuture<T> {
    type Output = T;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        // Delegate `poll` to the inner future
        self.0.as_mut().poll(cx)
    }
}

impl<T> FusedFuture for BlockingCallFuture<T> {
    fn is_terminated(&self) -> bool {
        self.0.is_terminated()
    }
}


impl<T> BlockingCallFuture<T> {
      
    /// Check if this long running blocking thread is done
    pub fn is_terminated(&self) -> bool {
        self.0.as_ref().is_terminated()
    }
    

    /// Races this future against a timeout, returning `Some(result)` if it completes first,
    /// or `None` if the timeout elapses.
    /// Keep calling this as needed to "wait" for the blocking call to complete.
    /// Does NOT stop early on shutdown so use caution in large durations
    pub async fn fetch(&mut self, timeout: Duration) -> Option<T> {
        let fut = &mut self.0;
        pin_mut!(fut); // Pin the mutable reference for polling in select!.

        // Assuming you're using something like tokio::time::sleep for the delay.
        // If using futures-timer or another crate, swap Delay::new with the equivalent.
        select! {
            _ = futures_timer::Delay::new(timeout).fuse() => None, // Timeout branch.
            result = fut => {
                // Your commented code had 'blocking_future = None;', but it's unclear what that refers to.
                // If it's external state, handle it here (e.g., set some atomic or mutex-guarded flag).
                Some(result)
            }
        }
    }


}

 
/// Trait for all steady actors, providing core actor and channel operations.
///
/// This trait is designed for single-threaded actor execution, and assumes
/// that all types passed between actors are `Send + Sync`.
#[allow(async_fn_in_trait)]
pub trait SteadyActor {
    /// Returns the frame rate in milliseconds for this actor.
    fn frame_rate_ms(&self) -> u64;

    /// which regeneration is this actor. starts at zero and goes up each time the actor is restarted.
    fn regeneration(&self) -> u32;

    /// Returns an optional reference to the Aeron media driver.
    fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>>;

    /// Runs the actor in a simulated environment with the provided simulation runners.
    async fn simulated_behavior(
        self,
        sims: Vec<&dyn IntoSimRunner<Self>>,
    ) -> Result<(), Box<dyn Error>>;

    /// Sets the log level for the entire application.
    fn loglevel(&self, loglevel: crate::LogLevel);

    /// Relays telemetry data to endpoints, throttling if called too frequently.
    ///
    /// Returns `true` if data was sent, `false` if throttled.
    fn relay_stats_smartly(&mut self) -> bool;

    /// Relays all collected telemetry data to endpoints, ignoring throttling.
    ///
    /// May overload telemetry if called too frequently.
    fn relay_stats(&mut self);

    /// Periodically relays telemetry data at the specified interval.
    ///
    /// Returns `true` if the full interval elapsed, `false` if interrupted by shutdown.
    async fn relay_stats_periodic(&mut self, duration_rate: Duration) -> bool;

    /// Checks if the actor's liveliness state matches any of the provided states.
    ///
    /// Returns `true` if a match is found.
    fn is_liveliness_in(&self, target: &[GraphLivelinessState]) -> bool;

    /// Returns `true` if the actor is in the "building" liveliness state.
    fn is_liveliness_building(&self) -> bool;

    /// Returns `true` if the actor is in the "running" liveliness state.
    fn is_liveliness_running(&self) -> bool;

    /// Returns `true` if the actor is in the "stop requested" liveliness state.
    fn is_liveliness_stop_requested(&self) -> bool;

    /// Returns an optional shutdown timeout duration if the actor is shutting down.
    fn is_liveliness_shutdown_timeout(&self) -> Option<Duration>;

    /// Flushes defragmented messages from the given transmitters and defrag state.
    ///
    /// Returns a tuple of (messages flushed, fragments flushed, optional error code).
    fn flush_defrag_messages<S: StreamControlItem>(
        &mut self,
        item: &mut Tx<S>,
        data: &mut Tx<u8>,
        defrag: &mut Defrag<S>,
    ) -> (u32, u32, Option<i32>);

    /// Waits for a consistent periodic interval between calls, accounting for work time.
    ///
    /// Returns `true` if the full interval elapsed, `false` if interrupted by shutdown.
    async fn wait_periodic(&self, duration_rate: Duration) -> bool;
    /// Waits for a fixed interval between calls, regardless of work time.
    ///
    /// Returns `true` if the full interval elapsed, `false` if interrupted by shutdown.
    async fn wait_timeout(&self, timeout: Duration) -> bool;


    /// Asynchronously waits for the specified duration.
    async fn wait(&self, duration: Duration);

    /// Waits until at least `count` units are available in the receiver.
    ///
    /// Returns `true` if available, `false` if interrupted.
    async fn wait_avail<T: RxCore>(&self, this: &mut T, size: usize) -> bool;

    /// Waits until at least `count` units are available in a bundle of receivers.
    ///
    /// Returns `true` if available, `false` if interrupted.
    async fn wait_avail_bundle<T: RxCore>(
        &self,
        this: &mut RxCoreBundle<'_, T>,
        size: usize,
        ready_channels: usize,
    ) -> bool;

    /// Waits for a future to complete or until shutdown is signaled.
    ///
    /// Returns `true` if the future completed, `false` if shutdown occurred.
    async fn wait_future_void<F>(&self, fut: F) -> bool
    where
        F: FusedFuture<Output = ()> + 'static + Send + Sync;

    /// Waits until at least `count` vacant units are available in the transmitter.
    ///
    /// Returns `true` if available, `false` if interrupted.
    async fn wait_vacant<T: TxCore>(&self, this: &mut T, count: T::MsgSize) -> bool;

    /// Waits until at least `count` vacant units are available in a bundle of transmitters.
    ///
    /// Returns `true` if available, `false` if interrupted.
    async fn wait_vacant_bundle<T: TxCore>(
        &self,
        this: &mut TxCoreBundle<'_, T>,
        count: T::MsgSize,
        ready_channels: usize,
    ) -> bool;

    /// Waits until a shutdown signal is received.
    ///
    /// Always returns `true`.
    async fn wait_shutdown(&self) -> bool;

    /// Peeks at the next available slice in the receiver without advancing the index.
    fn peek_slice<'b, T>(&self, this: &'b mut T) -> T::SliceSource<'b>
    where
        T: RxCore;

    /// Advances the take index in the receiver by the specified count.
    ///
    /// Returns the result of the operation.
    fn advance_take_index<T: RxCore>(&mut self, this: &mut T, count: T::MsgSize) -> RxDone;

    /// Takes a slice from the receiver into the provided target buffer.
    ///
    /// Returns the result of the operation.
    fn take_slice<T: RxCore>(
        &mut self,
        this: &mut T,
        target: T::SliceTarget<'_>,
    ) -> RxDone
    where
        T::MsgItem: Copy;

    /// Sends a slice from the provided source buffer into the transmitter.
    ///
    /// Returns the result of the operation.
    fn send_slice<T: TxCore>(
        &mut self,
        this: &mut T,
        source: T::SliceSource<'_>,
    ) -> TxDone
    where
        T::MsgOut: Copy;

    /// Peeks at the next available slice in the transmitter for writing.
    fn poke_slice<'b, T>(&self, this: &'b mut T) -> T::SliceTarget<'b>
    where
        T: TxCore;

    /// Advances the send index in the transmitter by the specified count.
    ///
    /// Returns the result of the operation.
    fn advance_send_index<T: TxCore>(&mut self, this: &mut T, count: T::MsgSize) -> TxDone;

    /// Attempts to peek at the next message in the receiver without removing it.
    ///
    /// Returns `Some(&T)` if a message is available, or `None` if empty.
    fn try_peek<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&'a T>;

    /// Returns an iterator over the messages currently in the receiver without removing them.
    fn try_peek_iter<'a, T>(
        &'a self,
        this: &'a mut Rx<T>,
    ) -> impl Iterator<Item = &'a T> + 'a;

    /// Checks if the receiver is currently empty.
    ///
    /// Returns `true` if empty, `false` otherwise.
    fn is_empty<T: RxCore>(&self, this: &mut T) -> bool;

    /// Returns the number of available units in the receiver.
    fn avail_units<T: RxCore>(&self, this: &mut T) -> T::MsgSize;

    /// Asynchronously peeks at the next available message in the receiver without removing it.
    ///
    /// Returns `Some` if a message is available, or `None` if the channel is closed.
    async fn peek_async<'a, T: RxCore>(
        &'a self,
        this: &'a mut T,
        ) -> Option<T::MsgPeek<'a>>;

    /// Sends messages from an iterator to the transmitter until it is full.
    ///
    /// Returns the number of messages successfully sent.
    fn send_iter_until_full<T, I: Iterator<Item = T>>(
        &mut self,
        this: &mut Tx<T>,
        iter: I,
    ) -> usize;

    /// Attempts to send a single message to the transmitter without blocking.
    ///
    /// Returns a `SendOutcome` indicating success or blockage.
    fn try_send<T: TxCore>(
        &mut self,
        this: &mut T,
        msg: T::MsgIn<'_>,
    ) -> SendOutcome<T::MsgOut>;

    /// Attempts to take a message from the receiver if available.
    ///
    /// Returns `Some(T)` if a message is available, or `None` if empty.
    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut>;

    /// Checks if the transmitter is currently full.
    ///
    /// Returns `true` if full, `false` otherwise.
    fn is_full<T: TxCore>(&self, this: &mut T) -> bool;

    /// Returns the number of vacant units in the transmitter.
    fn vacant_units<T: TxCore>(&self, this: &mut T) -> T::MsgSize;

    /// Asynchronously waits until the transmitter is empty.
    async fn wait_empty<T: TxCore>(&self, this: &mut T) -> bool;

    /// Takes all available messages from the receiver into an iterator.
    fn take_into_iter<'a, T: Sync + Send>(
        &mut self,
        this: &'a mut Rx<T>,
    ) -> impl Iterator<Item = T> + 'a;

    /// Calls an asynchronous function and monitors its execution for telemetry.
    ///
    /// Returns the output of the function, or `None` if interrupted.
    async fn call_async<F>(&self, operation: F) -> Option<F::Output>
    where
        F: Future;

   ///
   fn call_blocking<F, T>(&self, f: F) -> BlockingCallFuture<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static;

    /// Sends a message to the transmitter asynchronously, waiting if necessary for space.
    ///
    /// Returns a `SendOutcome` indicating success or blockage.
    async fn send_async<T: TxCore>(
        &mut self,
        this: &mut T,
        a: T::MsgIn<'_>,
        saturation: SendSaturation,
    ) -> SendOutcome<T::MsgOut>;

    /// Asynchronously takes a message from the receiver when available.
    ///
    /// Returns `Some(T)` if a message is available, or `None` if empty or shutdown.
    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T>;

    /// Asynchronously takes a message from the receiver, with a timeout.
    ///
    /// Returns `Some(T)` if a message is available, or `None` if empty, timed out, or shutdown.
    async fn take_async_with_timeout<T>(
        &mut self,
        this: &mut Rx<T>,
        timeout: Duration,
    ) -> Option<T>;

    /// Yields execution to allow other actors to run.
    ///
    /// Returns immediately if there is nothing scheduled.
    async fn yield_now(&self);

    /// Returns a side channel responder if one is available.
    fn sidechannel_responder(&self) -> Option<SideChannelResponder>;

    /// Checks if the actor is running, using a custom accept function.
    ///
    /// Returns `true` if running, `false` otherwise.
    fn is_running<F: FnMut() -> bool>(&mut self, accept_fn: F) -> bool;

    /// Requests a shutdown for the actor, awaiting any required barriers.
    async fn request_shutdown(&mut self);

    /// Retrieves the actor's arguments, cast to the specified type.
    ///
    /// Returns `Some(&A)` if available and of the correct type, or `None`.
    fn args<A: Any>(&self) -> Option<&A>;

    /// Retrieves the actor's identity.
    fn identity(&self) -> ActorIdentity;

    /// Checks if the current message in the receiver is a showstopper (peeked too many times).
    ///
    /// Returns `true` if the message has been peeked at least `threshold` times.
    fn is_showstopper<T>(&self, rx: &mut Rx<T>, threshold: usize) -> bool;
}

#[cfg(test)]
mod steady_actor_tests {
    use super::*;
    use crate::core_exec;

    #[test]
    fn test_send_outcome_methods() {
        let success: SendOutcome<i32> = SendOutcome::Success;
        assert!(success.is_sent());
        assert!(success.expect("should not panic"));

        let blocked = SendOutcome::Blocked(42);
        assert!(!blocked.is_sent());

        let timeout = SendOutcome::Timeout(42);
        assert!(!timeout.is_sent());

        let closed = SendOutcome::Closed(42);
        assert!(!closed.is_sent());
    }

    #[test]
    #[should_panic(expected = "test panic")]
    fn test_send_outcome_expect_panic() {
        let blocked = SendOutcome::Blocked(42);
        blocked.expect("test panic");
    }

    #[test]
    fn test_blocking_call_future_terminated() {
        let (tx, rx) = crate::oneshot::channel::<i32>();
        let mut fut = BlockingCallFuture(Box::pin(async move { rx.await.expect("") }.fuse()));
        assert!(!fut.is_terminated());
        tx.send(42).expect("send");
        let res = core_exec::block_on(&mut fut);
        assert_eq!(res, 42);
        assert!(fut.is_terminated());
    }

    #[test]
    fn test_blocking_call_future_fetch_timeout() {
        let (_tx, rx) = crate::oneshot::channel::<i32>();
        let mut fut = BlockingCallFuture(Box::pin(async move { rx.await.expect("") }.fuse()));
        let res = core_exec::block_on(fut.fetch(Duration::from_millis(10)));
        assert!(res.is_none());
    }

    #[test]
    fn test_blocking_call_future_fetch_ready() {
        let (tx, rx) = crate::oneshot::channel::<i32>();
        let mut fut = BlockingCallFuture(Box::pin(async move { rx.await.expect("") }.fuse()));
        tx.send(42).expect("send");
        let res = core_exec::block_on(fut.fetch(Duration::from_millis(100)));
        assert_eq!(res, Some(42));
    }
}
