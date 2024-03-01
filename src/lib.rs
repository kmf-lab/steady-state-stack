
pub(crate) mod telemetry {
    pub(crate) mod metrics_collector;
    pub(crate) mod metrics_server;
    pub(crate) mod setup;
}

pub(crate) mod serialize {
    pub(crate) mod byte_buffer_packer;
    pub(crate) mod fast_protocol_packed;
}
pub(crate) mod channel_stats;
pub(crate) mod config;
pub(crate) mod dot;
pub mod monitor;

pub mod channel_builder;
pub mod util;
pub mod serviced;
pub mod actor_builder;
mod actor_stats;
mod graph_testing;
mod graph_liveliness;

pub use graph_testing::GraphTestResult;
pub use actor_builder::SupervisionStrategy;
pub use graph_testing::EdgeSimulationDirector;
pub use monitor::LocalMonitor;
pub use channel_builder::Rate;
pub use channel_builder::Filled;
pub use actor_builder::MCPU;
pub use actor_builder::Work;
pub use actor_builder::Percentile;
pub use graph_liveliness::*;

use std::any::{Any, type_name};

use std::time::{Duration, Instant};

#[cfg(test)]
use std::collections::HashMap;

use std::fmt::Debug;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use futures::lock::Mutex;
use std::ops::{Deref, DerefMut};
use std::future::{Future, ready};
use std::pin::Pin;

use std::task::Context;

use std::thread::sleep;
use log::*;
use channel_builder::{InternalReceiver, InternalSender};
use ringbuf::producer::Producer;
use async_ringbuf::producer::AsyncProducer;
use ringbuf::traits::Observer;
use ringbuf::consumer::Consumer;
use async_ringbuf::consumer::AsyncConsumer;
use backtrace::Backtrace;

use nuclei::config::{IoUringConfiguration, NucleiConfig};
use actor_builder::ActorBuilder;
use crate::channel_builder::ChannelBuilder;
use crate::monitor::{ActorMetaData, ChannelMetaData, SteadyTelemetryActorSend, SteadyTelemetrySend};
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry::{metrics_collector, setup};
use crate::util::steady_logging_init;// re-publish in public
use futures::FutureExt; // Provides the .fuse() method

// Re-exporting Bastion blocking for use in actors
pub use bastion::blocking;
use futures::future::BoxFuture;


use futures::select;
use graph_liveliness::{ActorIdentity, GraphLiveliness, GraphLivelinessState};
use crate::graph_testing::EdgeSimulator;

pub type SteadyTx<T> = Arc<Mutex<Tx<T>>>;
pub type SteadyRx<T> = Arc<Mutex<Rx<T>>>;

pub type SteadyTxBundle<T,const GURTH:usize> = Arc<[Arc<Mutex<Tx<T>>>;GURTH]>;
pub type SteadyRxBundle<T,const GURTH:usize> = Arc<[Arc<Mutex<Rx<T>>>;GURTH]>;



/// Initialize logging for the steady_state crate.
/// This is a convenience function that should be called at the beginning of main.
pub fn init_logging(loglevel: &str) -> Result<(), Box<dyn std::error::Error>> {

    //TODO: should probably be its own init function.
    init_nuclei();  //TODO: add some configs and a boolean to control this?

    steady_logging_init(loglevel)
}

pub struct SteadyContext {
    pub(crate) ident: ActorIdentity,
    pub(crate) redundancy: usize,
    pub(crate) ctx: Option<Arc<bastion::context::BastionContext>>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    pub(crate) count_restarts: Arc<AtomicU32>,
    pub(crate) args: Arc<Box<dyn Any+Send+Sync>>,
    pub(crate) actor_metadata: Arc<ActorMetaData>,
}

impl SteadyContext {

    /// Sends a message to the channel asynchronously, waiting if necessary until space is available.
    ///
    /// # Parameters
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates that the message was successfully sent, and `Err(T)` if the send operation could not be completed.
    ///
    /// # Example Usage
    /// Suitable for scenarios where it's critical that a message is sent, and the sender can afford to wait.
    /// Not recommended for real-time systems where waiting could introduce unacceptable latency.
    pub async fn send_async<T>(& mut self, this: & mut Tx<T>, a: T,saturation_ok:bool) -> Result<(), T> {
        #[cfg(debug_assertions)]
        this.direct_use_check_and_warn();
        this.shared_send_async(a, self.ident,saturation_ok).await
    }

    pub fn edge_simulator(&self) -> Option<EdgeSimulator> {
        self.ctx.as_ref().map(|ctx| EdgeSimulator::new(ctx.clone()))
    }


    #[inline]
    pub fn is_running(&self, accept_fn: &mut dyn FnMut() -> bool) -> bool {
        match self.runtime_state.read() {
            Ok(liveliness) => {
                liveliness.is_running(self.ident, accept_fn)
            }
            Err(e) => {
                trace!("internal error,unable to get liveliness read lock {}",e);
                true
            }
        }
    }

    #[inline]
    pub fn request_graph_stop(&self) -> bool {
        match self.runtime_state.write() {
            Ok(mut liveliness) => {
                liveliness.request_shutdown();
                true
            }
            Err(e) => {
                trace!("internal error,unable to get liveliness write lock {}",e);
                false //keep running as the default under error conditions
            }
        }
    }

    #[inline]
    pub fn liveliness(&self) -> Arc<RwLock<GraphLiveliness>> {
        self.runtime_state.clone()
    }

    pub fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    pub fn identity(&self) -> ActorIdentity {
        self.ident
    }


    pub fn into_monitor<const RX_LEN: usize, const TX_LEN: usize>(self
                                                      , rx_mons: [& dyn RxDef; RX_LEN]
                                                      , tx_mons: [& dyn TxDef; TX_LEN]
    ) -> LocalMonitor<RX_LEN,TX_LEN> {

        //only build telemetry channels if this feature is enabled
        let (telemetry_send_rx, telemetry_send_tx, telemetry_state) = if config::TELEMETRY_HISTORY || config::TELEMETRY_SERVER {

            let mut rx_meta_data = Vec::new();
            let mut rx_inverse_local_idx = [0; RX_LEN];
            rx_mons.iter()
                .map(|rx| rx.meta_data())
                .enumerate()
                .for_each(|(c, md)| {
                    assert!(md.id < usize::MAX);
                    rx_inverse_local_idx[c]=md.id;
                    rx_meta_data.push(md);
                });

            let mut tx_meta_data = Vec::new();
            let mut tx_inverse_local_idx = [0; TX_LEN];
            tx_mons.iter()
                .map(|tx| tx.meta_data())
                .enumerate()
                .for_each(|(c, md)| {
                    assert!(md.id < usize::MAX);
                    tx_inverse_local_idx[c]=md.id;
                    tx_meta_data.push(md);
                });

            setup::construct_telemetry_channels(&self
                                                , rx_meta_data, rx_inverse_local_idx
                                                , tx_meta_data, tx_inverse_local_idx)
        } else {
            (None, None, None)
        };

        // this is my fixed size version for this specific thread
        LocalMonitor::<RX_LEN, TX_LEN> {
            telemetry_send_rx,
            telemetry_send_tx,
            telemetry_state,
            last_telemetry_send: Instant::now(),
            ident: self.ident,
            ctx: self.ctx,
            redundancy: self.redundancy,
            runtime_state: self.runtime_state,

            #[cfg(test)]
            test_count: HashMap::new(),
        }
    }



}

pub struct Graph  {
    pub(crate) args: Arc<Box<dyn Any+Send+Sync>>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) monitor_count: Arc<AtomicUsize>,
    //used by collector but could grow if we get new actors at runtime
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
}

impl Graph {

    /// needed for testing only, this monitor assumes we are running without a full graph
    /// and will not be used in production
    pub fn new_test_monitor(&mut self, name: & 'static str ) -> SteadyContext
    {
        // assert that we are NOT in release mode
        assert!(cfg!(debug_assertions), "This function is only for testing");

        let channel_count    = self.channel_count.clone();
        let all_telemetry_rx = self.all_telemetry_rx.clone();

        let count_restarts   = Arc::new(AtomicU32::new(0));
        SteadyContext {
            channel_count,
            ident: ActorIdentity{name, id: self.monitor_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst)},
            args: self.args.clone(),
            ctx: None, //this is key, we are not running in a graph by design
            actor_metadata: Arc::new(ActorMetaData::default()),
            redundancy: 1,
            all_telemetry_rx,
            runtime_state: self.runtime_state.clone(),
            count_restarts,
        }
    }
}


fn init_nuclei() {
    let nuclei_config = NucleiConfig {
        iouring: IoUringConfiguration::interrupt_driven(1 << 6),
        //iouring: IoUringConfiguration::kernel_poll_only(1 << 6),
        //iouring: IoUringConfiguration::low_latency_driven(1 << 6),
        //iouring: IoUringConfiguration::io_poll(1 << 6),
    };
    let _ = nuclei::Proactor::with_config(nuclei_config);

    if cfg!(target_arch = "wasm32") {
        //we can not write log files when in wasm
    } else {
        nuclei::spawn_blocking(|| {
            nuclei::drive(async {
                loop {
                    sleep(Duration::from_secs(10));
                    ready(()).await;
                };
            });
        }).detach();
    }

}

const MONITOR_UNKNOWN: usize = usize::MAX;
const MONITOR_NOT: usize = MONITOR_UNKNOWN-1; //any value below this is a valid monitor index

pub struct Tx<T> {
    pub(crate) tx: InternalSender<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize,
    pub(crate) last_error_send: Instant,
    pub(crate) is_closed: Arc<AtomicBool>, //only on shutdown
}

pub struct Rx<T> {
    pub(crate) rx: InternalReceiver<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize,
    pub(crate) is_closed: Arc<AtomicBool>, //only on shutdown
}

////////////////////////////////////////////////////////////////
impl<T> Tx<T> {

    ///the Rx should not expect any more messages than those already found
    ///on the channel. This is a signal that this actor has probably stopped
    pub fn mark_closed(&mut self) -> bool {
        self.is_closed.store(true, Ordering::SeqCst);
        true  //successful mark as closed
    }


    /// Returns the total capacity of the channel.
    /// This method retrieves the maximum number of messages the channel can hold.
    ///
    /// # Returns
    /// A `usize` indicating the total capacity of the channel.
    ///
    /// # Example Usage
    /// This method is useful for understanding the size constraints of the channel
    /// and for configuring buffer sizes or for performance tuning.

    pub fn capacity(&self) -> usize {
        self.shared_capacity()
    }

    /// Attempts to send a message to the channel without blocking.
    /// If the channel is full, the send operation will fail and return the message.
    ///
    /// # Parameters
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates successful send and `Err(T)` returns the message if the channel is full.
    ///
    /// # Example Usage
    /// Use this method for non-blocking send operations where immediate feedback on send success is required.
    /// Not suitable for scenarios where ensuring message delivery is critical without additional handling for failed sends.
    pub fn try_send(& mut self, msg: T) -> Result<(), T> {
        self.shared_try_send(msg)
    }

    /// Sends messages from an iterator to the channel until the channel is full.
    /// Each message is sent in a non-blocking manner.
    ///
    /// # Parameters
    /// - `iter`: An iterator that yields messages of type `T`.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    ///
    /// # Example Usage
    /// Ideal for batch sending operations where partial success is acceptable.
    /// Less suitable when all messages must be sent without loss, as this method does not guarantee all messages are sent.
    pub fn send_iter_until_full<I: Iterator<Item = T>>(&mut self, iter: I) -> usize {
        self.shared_send_iter_until_full(iter)
    }

    /// Sends a message to the channel asynchronously, waiting if necessary until space is available.
    ///
    /// # Parameters
    /// - `ident`: The identity of the actor sending the message. Get this from the SteadyContext.
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates that the message was successfully sent, and `Err(T)` if the send operation could not be completed.
    ///
    /// # Example Usage
    /// Suitable for scenarios where it's critical that a message is sent, and the sender can afford to wait.
    /// Not recommended for real-time systems where waiting could introduce unacceptable latency.
    pub async fn send_async(&mut self, ident:ActorIdentity, a: T, saturation_ok:bool) -> Result<(), T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_send_async(a, ident, saturation_ok).await
    }

    /// Sends messages from a slice to the channel until the channel is full.
    /// Operates in a non-blocking manner, similar to `send_iter_until_full`, but specifically for slices.
    /// This method requires `T` to implement the `Copy` trait, ensuring that messages can be copied into the channel without taking ownership.
    ///
    /// # Requirements
    /// - `T` must implement the `Copy` trait. This allows the method to efficiently handle the messages without needing to manage ownership or deal with borrowing complexities.
    ///
    /// # Parameters
    /// - `slice`: A slice of messages to be sent.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    ///
    /// # Example Usage
    /// Useful for efficiently sending multiple messages when partial delivery is sufficient.
    /// May not be appropriate for use cases requiring guaranteed delivery of all messages.
    ///
    /// # Pros and Cons of `T: Copy`
    /// ## Pros:
    /// - **Efficiency**: Allows for the fast and straightforward copying of messages into the channel, leveraging the stack when possible.
    /// - **Simplicity**: Simplifies message handling by avoiding the complexities of ownership and borrowing rules inherent in more complex types.
    /// - **Predictability**: Ensures that messages remain unchanged after being sent, providing clear semantics for message passing.
    ///
    /// ## Cons:
    /// - **Flexibility**: Restricts the types of messages that can be sent through the channel to those that implement `Copy`, potentially limiting the use of the channel with more complex data structures.
    /// - **Resource Use**: For types where `Copy` might involve deep copying (though typically `Copy` is for inexpensive-to-copy types), it could lead to unintended resource use.
    /// - **Design Constraints**: Imposes a design constraint on the types used with the channel, which might not always align with the broader application architecture or data handling strategies.
    ///
    /// By requiring `T: Copy`, this method optimizes for scenarios where messages are lightweight and can be copied without significant overhead, making it an excellent choice for high-throughput, low-latency applications.
    pub fn send_slice_until_full(&mut self, slice: &[T]) -> usize
       where T: Copy {
        self.shared_send_slice_until_full(slice)
    }

    /// Checks if the channel is currently full.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    ///
    /// # Example Usage
    /// Can be used to avoid calling `try_send` on a full channel, or to implement backpressure mechanisms.
    pub fn is_full(&self) -> bool {
        self.shared_is_full()
    }

    /// Returns the number of vacant units in the channel.
    /// Indicates how many more messages the channel can accept before becoming full.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    ///
    /// # Example Usage
    /// Useful for dynamically adjusting the rate of message sends or for implementing custom backpressure strategies.
    pub fn vacant_units(&self) -> usize {
        self.shared_vacant_units()
    }

    /// Asynchronously waits until at least a specified number of units are vacant in the channel.
    ///
    /// # Parameters
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Example Usage
    /// Use this method to delay message sending until there's sufficient space, suitable for scenarios where message delivery must be paced or regulated.
    pub async fn wait_vacant_units(&self, count: usize) {
        self.shared_wait_vacant_units(count).await
    }

    /// Asynchronously waits until the channel is empty.
    /// This method can be used to ensure that all messages have been processed before performing further actions.
    ///
    /// # Example Usage
    /// Ideal for scenarios where a clean state is required before proceeding, such as before shutting down a system or transitioning to a new state.
    pub async fn wait_empty(&self) {
        self.shared_wait_empty().await
    }



    //////////////////////////////////////////////////////////////////
    //////////////////////////////////////////////////////////////////

    fn direct_use_check_and_warn(&self) {
        if self.channel_meta_data.expects_to_be_monitored {
            write_warning_to_console(backtrace::Backtrace::new());

        }
    }
    ////////////////////////////////////////////////////////////////
    // Shared implmentations, if you need to swap out the channel it is done here
    ////////////////////////////////////////////////////////////////

    #[inline]
    fn shared_capacity(&self) -> usize {
        self.tx.capacity().get()
    }

    #[inline]
    fn shared_try_send(& mut self, msg: T) -> Result<(), T> {
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())}
            Err(m) => {Err(m)}
        }
    }
    #[inline]
    fn shared_send_iter_until_full<I: Iterator<Item = T>>(&mut self, iter: I) -> usize {
        self.tx.push_iter(iter)
    }
    #[inline]
    fn shared_send_slice_until_full(&mut self, slice: &[T]) -> usize
        where T: Copy {
        self.tx.push_slice(slice)
    }
    #[inline]
    fn shared_is_full(&self) -> bool {
        self.tx.is_full()
    }
    #[inline]
    fn shared_vacant_units(&self) -> usize {
        self.tx.vacant_len()
    }
    #[inline]
    async fn shared_wait_vacant_units(&self, count: usize) {
        self.tx.wait_vacant(count).await
    }

    #[inline]
    async fn shared_wait_empty(&self) {
        self.tx.wait_vacant(usize::from(self.tx.capacity())).await
    }

    #[inline]
    async fn shared_send_async(& mut self, msg: T, ident: ActorIdentity, saturation_ok: bool) -> Result<(), T> {
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())},
            Err(msg) => {
                //caller can decide this is ok and not a warning
                if !saturation_ok {
                    //NOTE: we slow the rate of errors reported
                    // TODO: this "slow" rate should be a happy little macro done everywhere
                    if self.last_error_send.elapsed().as_secs() > 10 {
                        let type_name = type_name::<T>().split("::").last();
                        warn!("{:?} tx full channel #{} {:?} cap:{:?} type:{:?} "
                                   , ident
                                   , self.channel_meta_data.id
                                   , self.channel_meta_data.labels
                                   , self.tx.capacity()
                                   , type_name);
                        self.last_error_send = Instant::now();
                    }
                    // here we will wait until there is room in the channel
                }
                //TODO: a timeout here may be a good idea also abandon on shutdown

                select! {
                    _ = self.tx.wait_vacant(1).fuse() => {
                        match self.tx.push(msg).await {
                            Ok(_) => {Ok(())}
                            Err(_) => { //should not happen, internal error
                                error!("channel is closed");
                                Ok(())
                            }
                        }

                    }
                }

            }
        }
    }


}

fn write_warning_to_console(stack: Backtrace) {
    warn!("you called this without the monitor but monitoring for this channel is enabled. see the monitor version of this method");

    let mut count:usize = 0;
    for frame in stack.frames() {
        let symbols = frame.symbols();
        for symbol in symbols {
            if let Some(filename) = symbol.filename() {
                let filename_str = filename.to_string_lossy();
                // Filter based on a marker in the path
                // Replace "my_project_marker" with something that uniquely identifies your project
                // if filename_str.contains("steady") {
                // Print only the relevant part of the stack
                if let Some(name) = symbol.name() {
                    //hide the fuse and pin stuff which will distract from finding the issue
                    if   !name.to_string().contains("fuse::Fuse<Fut>")
                      && !name.to_string().contains("core::pin::Pin")
                      && !name.to_string().contains("future::FutureExt")
                    {
                        if count > 0 {
                            println!("{} {:?} \n   at {}:{}", count, name, filename_str, symbol.lineno().unwrap_or(0));
                        }
                        count += 1;
                    }

                   if 3 == count {
                       return;
                   }
                }
                //  }
            }
        }
    }

}


impl<T> Rx<T> {

    /// only for use in unit tests
    pub fn block_until_not_empty(&self, duration: Duration) {
        assert!(cfg!(debug_assertions), "This function is only for testing");
        let start = Instant::now();
        loop {
            if !self.is_empty() {
                break;
            }
            if start.elapsed() > duration {
                error!("timeout waiting for channel to not be empty");
                break;
            }
            std::thread::yield_now();
        }
    }


    pub fn is_closed(&self) -> bool {
        self.is_closed.load(Ordering::SeqCst)
    }

    /// Returns the total capacity of the channel.
    /// This method retrieves the maximum number of messages the channel can hold.
    ///
    /// # Returns
    /// A `usize` indicating the total capacity of the channel.
    ///
    /// # Example Usage
    /// Useful for initial configuration and monitoring of channel capacity to ensure it aligns with expected load.
    pub fn capacity(&self) -> usize {
        self.shared_capacity()
    }

    /// Attempts to peek at a slice of messages from the channel without removing them.
    /// This operation is non-blocking and allows multiple messages to be inspected simultaneously.
    ///
    /// Requires T: Copy but this is still up for debate as it is not clear if this is the best way to handle this
    ///
    /// # Parameters
    /// - `elems`: A mutable slice to store the peeked messages. Its length determines the maximum number of messages to peek.
    ///
    /// # Returns
    /// The number of messages actually peeked and stored in `elems`.
    ///
    /// # Example Usage
    /// Ideal for scenarios where processing or decision making is based on a batch of incoming messages without consuming them.
    pub fn try_peek_slice(&self, elems: &mut [T]) -> usize
           where T: Copy {
           self.shared_try_peek_slice(elems)
    }

    /// Asynchronously waits to peek at a slice of messages from the channel without removing them.
    /// Waits until the specified number of messages is available or the channel is closed.
    ///
    /// Requires T: Copy but this is still up for debate as it is not clear if this is the best way to handle this
    ///
    /// # Parameters
    /// - `wait_for_count`: The number of messages to wait for before peeking.
    /// - `elems`: A mutable slice to store the peeked messages.
    ///
    /// # Returns
    /// The number of messages actually peeked and stored in `elems`.
    ///
    /// # Example Usage
    /// Suitable for use cases requiring a specific number of messages to be available for batch processing.
    pub async fn peek_async_slice(&mut self, wait_for_count: usize, elems: &mut [T]) -> usize
       where T: Copy {
        self.shared_peek_async_slice(wait_for_count, elems).await
    }

    /// Retrieves and removes a slice of messages from the channel.
    /// This operation is blocking and will remove the messages from the channel.
    /// Note: May take fewer messages than the slice length if the channel is not full.
    /// This method requires `T` to implement the `Copy` trait, ensuring efficient handling of message data.
    ///
    /// # Requirements
    /// - `T` must implement the `Copy` trait. This constraint ensures that messages can be efficiently copied out of the channel without ownership issues.
    ///
    /// # Parameters
    /// - `elems`: A mutable slice where the taken messages will be stored.
    ///
    /// # Returns
    /// The number of messages actually taken and stored in `elems`.
    ///
    /// # Example Usage
    /// Useful for batch processing where multiple messages are consumed at once for efficiency. Particularly effective in scenarios where processing overhead needs to be minimized.
    ///
    /// # Pros and Cons of `T: Copy`
    /// ## Pros:
    /// - **Performance Optimization**: Facilitates quick, lightweight operations by leveraging the ability to copy data directly, without the overhead of more complex memory management.
    /// - **Simplicity in Message Handling**: Reduces the complexity of managing message lifecycles and ownership, streamlining channel operations.
    /// - **Reliability**: Ensures that the operation does not inadvertently alter or lose message data during transfer, maintaining message integrity.
    ///
    /// ## Cons:
    /// - **Type Limitation**: Limits the use of the channel to data types that implement `Copy`, potentially excluding more complex or resource-managed types that might require more nuanced handling.
    /// - **Overhead for Larger Types**: While `Copy` is intended for lightweight types, using it with larger, though still `Copy`, data types could introduce unnecessary copying overhead.
    /// - **Design Rigidity**: Imposes a strict design requirement on the types that can be used with the channel, which might not align with all application designs or data models.
    ///
    /// Incorporating the `T: Copy` requirement, `take_slice` is optimized for use cases prioritizing efficiency and simplicity in handling a series of lightweight messages, making it a key method for high-performance message processing tasks.
    pub fn take_slice(&mut self, elems: &mut [T]) -> usize
        where T: Copy {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_take_slice(elems)
    }

    /// Attempts to take a single message from the channel without blocking.
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` if a message is available, or `None` if the channel is empty.
    ///
    /// # Example Usage
    /// Ideal for non-blocking single message consumption, allowing for immediate feedback on message availability.
    pub fn try_take(& mut self) -> Option<T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_try_take()
    }

    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    /// # Returns
    /// A `Result<T, String>`, where `Ok(T)` is the message if available, and `Err(String)` contains an error message if the retrieval fails.
    ///
    /// # Example Usage
    /// Suitable for async processing where messages are consumed one at a time as they become available.
    pub async fn take_async(& mut self) -> Result<T,String> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_take_async().await
    }

    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message is available, or `None` if the channel is empty.
    ///
    /// # Example Usage
    /// Can be used to inspect the next message without consuming it, useful in decision-making scenarios.
    pub fn try_peek(&self) -> Option<&T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_try_peek()
    }

    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Example Usage
    /// Enables iterating over messages for inspection or conditional processing without consuming them.
    pub fn try_peek_iter(& self) -> impl Iterator<Item = & T>  {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_try_peek_iter()
    }

    /// Asynchronously peeks at the next message in the channel without removing it.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message becomes available, or `None` if the channel is closed.
    ///
    /// # Example Usage
    /// Useful for async scenarios where inspecting the next message without consuming it is required.
    pub async fn peek_async(& mut self) -> Option<&T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_peek_async().await
    }

    /// Asynchronously returns an iterator over the messages in the channel, waiting for a specified number of messages to be available.
    ///
    /// # Parameters
    /// - `wait_for_count`: The number of messages to wait for before returning the iterator.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Example Usage
    /// Ideal for async batch processing where a specific number of messages are needed for processing.
    pub async fn peek_async_iter(& mut self, wait_for_count: usize) -> impl Iterator<Item = & T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_peek_async_iter(wait_for_count).await
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    ///
    /// # Example Usage
    /// Useful for determining if the channel is empty before attempting to consume messages.
    pub fn is_empty(&self) -> bool {
        //not async and immutable so no need to check
        self.shared_is_empty()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    ///
    /// # Example Usage
    /// Enables monitoring of the current load or backlog of messages in the channel for adaptive processing strategies.
    pub fn avail_units(& mut self) -> usize {
        //not async and immutable so no need to check
        self.shared_avail_units()
    }

    /// Asynchronously waits for a specified number of messages to be available in the channel.
    ///
    /// # Parameters
    /// - `count`: The number of messages to wait for.
    ///
    /// # Example Usage
    /// Suitable for scenarios requiring batch processing where a certain number of messages are needed before processing begins.
    #[inline]
    pub async fn wait_avail_units(& mut self, count: usize) {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_wait_avail_units(count).await
    }
    //////////////////////////////////////////////////////////////////
    //////////////////////////////////////////////////////////////////

    fn direct_use_check_and_warn(&self) {
        if self.channel_meta_data.expects_to_be_monitored {
            write_warning_to_console(backtrace::Backtrace::new());

        }
    }

    ///////////////////////////////////////////////////////////////////
    // these are the shared internal private implementations
    // if you want to swap out the channel implementation you can do it here
    ///////////////////////////////////////////////////////////////////

    #[inline]
    fn shared_capacity(&self) -> usize {
        self.rx.capacity().get()
    }

    #[inline]
    fn shared_try_peek_slice(&self, elems: &mut [T]) -> usize
      // TODO: rewrite
        where T: Copy {
        let mut last_index = 0;
        for (i, e) in self.rx.iter().enumerate() {
            if i < elems.len() {
                elems[i] = *e; // Assuming e is a reference and needs dereferencing
                last_index = i;
            } else {
                break;
            }
        }
        // Return the count of elements written, adjusted for 0-based indexing
        last_index + 1
    }

    #[inline]
    async fn shared_peek_async_slice(&mut self, wait_for_count: usize, elems: &mut [T]) -> usize
        where T: Copy {
        self.rx.wait_occupied(wait_for_count).await;
        self.shared_try_peek_slice(elems)
    }

    #[inline]
    fn shared_take_slice(&mut self, elems: &mut [T]) -> usize
        where T: Copy {
        self.rx.pop_slice(elems)
    }


    #[inline]
    async fn shared_take_async(& mut self) -> Result<T,String> {
        // implementation favors a full channel
        if let Some(m) = self.rx.try_pop() {
            Ok(m)
        } else {
            match self.rx.pop().await {
                Some(a) => { Ok(a) }
                None => { Err("Producer is dropped".to_string()) }
            }
        }
    }

    #[inline]
    fn shared_try_peek_iter(& self) -> impl Iterator<Item = & T>  {
        self.rx.iter()
    }

    #[inline]
    async fn shared_peek_async(& mut self) -> Option<&T> {
        self.rx.wait_occupied(1).await;
        self.rx.first()
    }

    #[inline]
    async fn shared_peek_async_iter(& mut self, wait_for_count: usize) -> impl Iterator<Item = & T> {
        self.rx.wait_occupied(wait_for_count).await;
        self.rx.iter()
    }

    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.rx.is_empty()
    }
    #[inline]
    fn shared_avail_units(& mut self) -> usize {
        self.rx.occupied_len()
    }

    #[inline]
    async fn shared_wait_avail_units(& mut self, count: usize) {
        //TODO: we need a timeout here as well for better control
        self.rx.wait_occupied(count).await
    }

    #[inline]
    fn shared_try_take(& mut self) -> Option<T> {
        self.rx.try_pop()
    }

    #[inline]
    fn shared_try_peek(&self) -> Option<&T> {
        self.rx.first()
    }

}


pub trait TxDef: Debug {
    fn meta_data(&self) -> Arc<ChannelMetaData>;
}
pub trait RxDef: Debug + Send {
    fn meta_data(&self) -> Arc<ChannelMetaData>;
    fn wait_avail_units(&self, count: usize) -> BoxFuture<'_, ()>;

    }

//BoxFuture

impl <T> TxDef for SteadyTx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        let guard = bastion::run!(self.lock());
        let this = guard.deref();
        this.channel_meta_data.clone()
    }
}

impl <T: std::marker::Send> RxDef for SteadyRx<T>  {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        let guard = bastion::run!(self.lock());
        let this = guard.deref();
        this.channel_meta_data.clone()
    }

    #[inline]
    fn wait_avail_units(&self, count: usize) -> BoxFuture<'_, ()> {
        async move {
            let mut guard = self.lock().await;
            guard.deref_mut().shared_wait_avail_units(count).await;
        }.boxed() // Use the `.boxed()` method to convert the future into a BoxFuture
    }


}



pub struct SteadyBundle{}

impl SteadyBundle {

    pub fn mark_closed<T, const GIRTH: usize>(this: & SteadyTxBundle<T, GIRTH>) -> bool {
        this.iter().all(|tx| {
            let mut guard = bastion::run!(tx.lock());
            guard.deref_mut().mark_closed()
        })
    }

    pub fn is_closed<T, const GIRTH: usize>(this: & SteadyRxBundle<T, GIRTH>) -> bool {
        this.iter().all(|rx| {
            let mut guard = bastion::run!(rx.lock());
            guard.deref_mut().is_closed()
        })
    }


    pub fn tx_def_slice<T, const GIRTH: usize>(this: & SteadyTxBundle<T, GIRTH>) -> [& dyn TxDef; GIRTH] {
        this.iter()
            .map(|x| x as &dyn TxDef)
            .collect::<Vec<&dyn TxDef>>()
            .try_into()
            .expect("Internal Error")
    }
    pub fn tx_new_bundle<T, const GIRTH: usize>(txs: Vec<SteadyTx<T>>) -> SteadyTxBundle<T, GIRTH> {
        let result: [SteadyTx<T>; GIRTH] = txs.try_into().expect("Incorrect length");
        Arc::new(result)
    }


    pub fn rx_def_slice< T, const GIRTH: usize>(this: & SteadyRxBundle<T, GIRTH>) -> [& dyn RxDef; GIRTH]
     where T: Send {
        this.iter()
            .map(|x| x as &dyn RxDef)
            .collect::<Vec<&dyn RxDef>>()
            .try_into()
            .expect("Internal Error")
    }
    pub fn rx_new_bundle<T, const GIRTH: usize>(rxs: Vec<SteadyRx<T>>) -> SteadyRxBundle<T, GIRTH> {
        let result: [SteadyRx<T>; GIRTH] = rxs.try_into().expect("Incorrect length");
        Arc::new(result)
    }

    pub fn new_bundles<T, const GIRTH: usize>(base_channel_builder: &ChannelBuilder) -> (SteadyTxBundle<T,GIRTH>, SteadyRxBundle<T,GIRTH>) {

        // Initialize vectors to hold the separate components
        let mut tx_vec: Vec<SteadyTx<T>> = Vec::with_capacity(GIRTH);
        let mut rx_vec: Vec<SteadyRx<T>> = Vec::with_capacity(GIRTH);

        (0..GIRTH).for_each(|_| {
            let (t,r) = base_channel_builder.build();
            tx_vec.push(t);
            rx_vec.push(r);
        });

        (SteadyBundle::tx_new_bundle::<T, GIRTH>(tx_vec), SteadyBundle::rx_new_bundle::<T, GIRTH>(rx_vec))

    }


}


#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StdDev(f32);

impl StdDev {
    // Private constructor to directly set the value inside the struct.
    // This is private to ensure that all public constructors go through validation.
    fn new(value: f32) -> Option<Self> {
        if value > 0.0 && value < 10.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub fn one() -> Self {
        Self(1.0)
    }

    pub fn one_and_a_half() -> Self {
        Self(1.5)
    }

    pub fn two() -> Self {
        Self(2.0)
    }

    pub fn two_and_a_half() -> Self {
        Self(2.5)
    }

    pub fn three() -> Self {
        Self(3.0)
    }

    pub fn four() -> Self {
        Self(4.0)
    }

    // Allows custom values within the valid range.
    pub fn custom(value: f32) -> Option<Self> {
        Self::new(value)
    }

    // Getter to access the inner f32 value.
    pub fn value(&self) -> f32 {
        self.0
    }
}

////////////////

////////////////////
pub trait Metric {
}
pub trait DataMetric: Metric {
}
pub trait ComputeMetric: Metric {
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertColor {
    Yellow,
    Orange,
    Red,
}

impl Metric for Duration {}

#[derive(Clone, Copy, Debug)]
pub enum Trigger<T> where T: Metric {
    AvgAbove(T),
    AvgBelow(T),
    StdDevsAbove(StdDev, T), // above mean+(std*factor)
    StdDevsBelow(StdDev, T), // below mean-(std*factor)
    PercentileAbove(Percentile, T),
    PercentileBelow(Percentile, T),
}

///////////////////////////////

