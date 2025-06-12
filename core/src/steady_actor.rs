
use std::future::Future;
use std::time::{Duration, Instant};
use futures_util::future::FusedFuture;
use std::any::Any;
use std::error::Error;
use std::sync::{Arc};
use aeron::aeron::Aeron;
use futures_util::lock::Mutex;
use crate::{steady_config, ActorIdentity, GraphLivelinessState, Rx, RxCoreBundle, SendSaturation, Tx, TxCoreBundle};
use crate::graph_testing::SideChannelResponder;
use crate::monitor::{RxMetaData, TxMetaData};
use crate::monitor_telemetry::SteadyTelemetry;
use crate::steady_rx::RxMetaDataProvider;
use crate::steady_tx::{TxDone, TxMetaDataProvider};
use crate::telemetry::setup;
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::steady_actor_spotlight::SteadyActorSpotlight;
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::distributed::distributed_stream::{Defrag, StreamItem};
use crate::simulate_edge::{IntoSimRunner};

impl SteadyActorShadow {
    /// Converts the context into a local monitor.
    ///
    /// # Parameters
    /// - `rx_mons`: Array of receiver monitors.
    /// - `tx_mons`: Array of transmitter monitors.
    ///
    /// # Returns
    /// A `LocalMonitor` instance.
    pub fn into_spotlight<const RX_LEN: usize, const TX_LEN: usize>(
        self,
        rx_mons: [&dyn RxMetaDataProvider; RX_LEN], //todo T: RxDef and TxDef
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
    /// Internal method to convert the context into a local monitor.
    ///
    /// # Parameters
    /// - `rx_mons`: Array of receiver metadata.
    /// - `tx_mons`: Array of transmitter metadata.
    ///
    /// # Returns
    /// A `LocalMonitor` instance.
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
                rx_inverse_local_idx[c] = md.id;
                rx_meta_data.push(md.clone());
            });

            let mut tx_meta_data = Vec::new();
            let mut tx_inverse_local_idx = [0; TX_LEN];
            tx_mons.iter().enumerate().for_each(|(c, md)| {
                assert!(md.id < usize::MAX);
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
            show_thread_info: self.show_thread_info,
            team_id: self.team_id,
            is_running_iteration_count: 0,
            aeron_meda_driver: self.aeron_meda_driver.clone(),
            use_internal_behavior: self.use_internal_behavior,
            shutdown_barrier: self.shutdown_barrier.clone(),
        }
    }
}

#[derive(Default)]
pub enum SendOutcome<X> {
    #[default]
    Success,
    Blocked(X),
}

impl<X> SendOutcome<X> {
    /// Returns `true` if the write was successful (i.e., the enum is `Written`), otherwise `false`.
    pub fn is_sent(&self) -> bool {
        match self {
            SendOutcome::Success => true,
            SendOutcome::Blocked(_) => false,
        }
    }

    pub fn expect(self, msg: &'static str) -> bool {
        match self {
            SendOutcome::Success => true,
            SendOutcome::Blocked(_) => panic!("{}",msg),
        }
    }
}

/// NOTE this trait is passed into actors and actors are tied to a single thread. As a result
///      we need not worry about these methods needing Send. We also know that T will come
///      from other actors so we can assume that T is Send + Sync
#[allow(async_fn_in_trait)]
pub trait SteadyActor {

    fn frame_rate_ms(&self) -> u64;

    fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>>;

    async fn simulated_behavior(self, sims: Vec<&dyn IntoSimRunner<Self>>) -> Result<(), Box<dyn Error>>;

        /// set log level for the entire application
    fn loglevel(&self, loglevel: crate::LogLevel);

    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    /// NOTE: This does NOTHING unless called on the LocalMonitor instance
    ///
    /// This method holds the data if it is called more frequently than the collector can consume the data.
    /// It is designed for use in tight loops where telemetry data is collected frequently.
    fn relay_stats_smartly(&mut self) -> bool;

    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    /// NOTE: This does NOTHING unless called on the LocalMonitor instance
    ///
    /// This method ignores the last telemetry send time and may overload the telemetry if called too frequently.
    /// It is designed for use in low-frequency telemetry collection scenarios, specifically cases
    /// when we know the actor will do a blocking wait for a long time and we want relay what we have before that.
    fn relay_stats(&mut self);

    /// Periodically relays telemetry data at a specified rate.
    /// NOTE: This does NOTHING unless called on the LocalMonitor instance
    ///
    /// # Parameters
    /// - `duration_rate`: The interval at which telemetry data should be sent.
    ///
    /// # Returns
    /// `true` if the full waiting duration was completed without interruption.
    /// `false` if a shutdown signal was detected during the waiting period.
    ///
    /// # Asynchronous
    async fn relay_stats_periodic(&mut self, duration_rate: Duration) -> bool;


    /// Checks if the liveliness state matches any of the target states.
    ///
    /// # Parameters
    /// - `target`: A slice of `GraphLivelinessState`.
    ///
    /// # Returns
    /// `true` if the liveliness state matches any target state, otherwise `false`.
    fn is_liveliness_in(&self, target: &[GraphLivelinessState]) -> bool;
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_building(&self) -> bool;
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_running(&self) -> bool;
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_stop_requested(&self) -> bool;
    /// if we are shutting down return the optional timeout before we force the exit
    fn is_liveliness_shutdown_timeout(&self) -> Option<Duration>;

    fn flush_defrag_messages<S: StreamItem>(&mut self
                                            , item: &mut Tx<S>
                                            , data: &mut Tx<u8>
                                            , defrag: &mut Defrag<S>
    ) -> (u32,u32,Option<i32>);


    /// Waits for a specified duration, ensuring a consistent periodic interval between calls.
    ///
    /// This method helps maintain a consistent period between consecutive calls, even if the
    /// execution time of the work performed in between calls fluctuates. It calculates the
    /// remaining time until the next desired periodic interval and waits for that duration.
    ///
    /// If a shutdown signal is detected during the waiting period, the method returns early
    /// with a value of `false`. Otherwise, it waits for the full duration and returns `true`.
    ///
    /// # Arguments
    ///
    /// * `duration_rate` - The desired duration between periodic calls.
    ///
    /// # Returns
    ///
    /// * `true` if the full waiting duration was completed without interruption.
    /// * `false` if a shutdown signal was detected during the waiting period.
    ///
    async fn wait_periodic(&self, duration_rate: Duration) -> bool;


    /// Asynchronously waits for a specified duration.
    ///
    /// # Parameters
    /// - `duration`: The duration to wait.
    ///
    /// # Asynchronous
    async fn wait(&self, duration: Duration);
    
    async fn wait_avail<T: RxCore>(&self, this: &mut T, count: usize) -> bool;


//TODO: soon
//    async fn wait_avail_with_timeout<T: RxCore>(&self, this: &mut T, count: usize, timeout: Duration) -> bool;



    async fn wait_avail_bundle<T: RxCore>(&self, this: &mut RxCoreBundle<'_, T>, count: usize, ready_channels: usize) -> bool;


    /// Waits for a future to complete or until a shutdown signal is received.
    ///
    /// # Parameters
    /// - `fut`: The future to wait for.
    ///
    /// # Returns
    /// `true` if the future completed, `false` if a shutdown signal was received.
    async fn wait_future_void<F>(&self, fut: F) -> bool
    where
        F: FusedFuture<Output = ()> + 'static + Send + Sync;



    /// Waits until the specified number of vacant units are in the transmitter.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    async fn wait_vacant<T: TxCore>(&self, this: &mut T, count: T::MsgSize) -> bool;

    async fn wait_vacant_bundle<T: TxCore>(&self, this: &mut TxCoreBundle<'_, T>, count: T::MsgSize, ready_channels: usize) -> bool;




    /// Waits until shutdown
    ///
    /// # Returns
    /// true
    async fn wait_shutdown(&self) -> bool;






    /// will call shared_peek_slice  
    fn peek_slice<T>(&self, this: &mut Rx<T>, elems: &mut [T]) -> usize
    where
        T: Copy;
    /// will call shared_take_slice
    fn take_slice<T>(&mut self, this: &mut Rx<T>, slice: &mut [T]) -> usize
    where
        T: Copy,
    ;

    /// first done, calls shared_send_slice
    fn send_slice<'b,T: TxCore>(&'b mut self, this: &'b mut T, slice: T::SliceSource<'b>) -> TxDone
    where
        T::MsgOut : Copy;

    // will call shared_send_direct
    // new method   fn send_slice_direct <'b,T: TxCore>(&'b mut self, this: &'b mut T, slice: T::SliceSource<'b>) -> TxDone
    //                       where
    //                             T::MsgOut : Copy;






    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message is available, or `None` if the channel is empty.
    fn try_peek<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&'a T>
    ;
    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    fn try_peek_iter<'a, T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item=&'a T> + 'a;
    
    /// Checks if the channel is currently empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    fn is_empty<T: RxCore>(&self, this: &mut T) -> bool;
    /// Returns the number of messages currently available in the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    fn avail_units<T: RxCore>(&self, this: &mut T) -> usize;
    /// Asynchronously peeks at the next available message in the channel without removing it.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message becomes available, or `None` if the channel is closed.
    ///
    /// # Asynchronous
    async fn peek_async<'a, T: RxCore>(&'a self, this: &'a mut T) -> Option<T::MsgPeek<'a>>;


    /// Sends messages from an iterator to the Tx channel until it is full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `iter`: An iterator that yields messages of type `T`.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    fn send_iter_until_full<T, I: Iterator<Item=T>>(&mut self, this: &mut Tx<T>, iter: I) -> usize;
    /// Attempts to send a single message to the Tx channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A SendOutcome<T>
    fn try_send<T: TxCore>(&mut self, this: &mut T, msg: T::MsgIn<'_>) -> SendOutcome<T::MsgOut>;

    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut>;

    /// Checks if the Tx channel is currently full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    fn is_full<T: TxCore>(&self, this: &mut T) -> bool;
    /// Returns the number of vacant units in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    fn vacant_units<T: TxCore>(&self, this: &mut T) -> usize;
    /// Asynchronously waits until the Tx channel is empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Asynchronous
    async fn wait_empty<T: TxCore>(&self, this: &mut T) -> bool;
    /// Takes messages into an iterator.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the taken messages.
    fn take_into_iter<'a, T: Sync + Send>(&mut self, this: &'a mut Rx<T>) -> impl Iterator<Item=T> + 'a;
    /// Calls an asynchronous function and monitors its execution for telemetry.
    ///
    /// # Parameters
    /// - `f`: The asynchronous function to call.
    ///
    /// # Returns
    /// The output of the asynchronous function `f`.
    ///
    /// # Asynchronous
    async fn call_async<F>(&self, operation: F) -> Option<F::Output>
    where
        F: Future,
    ;

    /// Sends a message to the channel asynchronously, waiting if necessary until space is available.
    ///
    /// # Parameters
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A SendOutcome<T>
    ///
    /// # Example Usage
    /// Suitable for scenarios where it's critical that a message is sent, and the sender can afford to wait.
    /// Not recommended for real-time systems where waiting could introduce unacceptable latency.
    async fn send_async<T: TxCore>(&mut self, this: &mut T, a: T::MsgIn<'_>, saturation: SendSaturation) -> SendOutcome<T::MsgOut>;


    /// Move the read index forward count items, ie jump over these
    fn advance_read_index<T>(&mut self, this: &mut Rx<T>, count: usize) -> usize;


    /// Attempts to take a message from the channel when available.
    /// Returns early if shutdown signal is detected
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T>;

    /// Attempts to take a message from the channel if available. Provides a timeout to wait.
    /// Returns early if shutdown signal is detected
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    async fn take_async_with_timeout<T>(&mut self, this: &mut Rx<T>, timeout: Duration) -> Option<T>;



    /// Yield so other actors may be able to make use of this thread. Returns
    /// immediately if there is nothing scheduled to check.
    async fn yield_now(&self);
    

    /// Returns a side channel responder if available.
    ///
    /// # Returns
    /// An `Option` containing a `SideChannelResponder` if available.
    fn sidechannel_responder(&self) -> Option<SideChannelResponder>;
    /// Checks if the actor is running, using a custom accept function.
    ///
    /// # Parameters
    /// - `accept_fn`: The custom accept function to check the running state.
    ///
    /// # Returns
    /// `true` if the actor is running, `false` otherwise.
    fn is_running<F: FnMut() -> bool>(&mut self, accept_fn: F) -> bool;
    /// Requests a graph stop for the actor.
    /// will await if a barrier is in place needing more actor approvals
    /// for the simple case will return immediately upon changing the state
    async fn request_shutdown(&mut self);  // see with_graph_stop_barrier_count

    /// Retrieves the actor's arguments, cast to the specified type.
    ///
    /// # Returns
    /// An `Option<&A>` containing the arguments if available and of the correct type.
    fn args<A: Any>(&self) -> Option<&A>;
    /// Retrieves the actor's identity.
    ///
    /// # Returns
    /// An `ActorIdentity` representing the actor's identity.
    fn identity(&self) -> ActorIdentity;

    /// Checks if the current message in the receiver is a showstopper (peeked N times without being taken).
    /// If true you should consider pulling this message for a DLQ or log it or consider dropping it.
    fn is_showstopper<T>(&self, rx: &mut Rx<T>, threshold: usize) -> bool;

}
