
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
pub(crate) mod monitor;

pub mod channel_builder;
pub mod util;
pub mod serviced;
pub mod actor_builder;
mod actor_stats;


use std::any::{Any, type_name};
use std::backtrace::Backtrace;

#[cfg(test)]
use std::collections::HashMap;

use std::fmt::Debug;
//re-publish bastion from steady_state for this early version
pub use bastion;
use std::time::{Duration, Instant};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use futures::lock::Mutex;
use std::ops::{Deref, DerefMut, Sub};
use std::future::{Future, ready};
use std::process::exit;
use std::thread::sleep;
use log::*;
use channel_builder::{InternalReceiver, InternalSender};
use ringbuf::producer::Producer;
use async_ringbuf::producer::AsyncProducer;
use ringbuf::traits::Observer;
use ringbuf::consumer::Consumer;
use async_ringbuf::consumer::AsyncConsumer;
use futures_timer::Delay;
use nuclei::config::{IoUringConfiguration, NucleiConfig};
use actor_builder::ActorBuilder;
use crate::channel_builder::ChannelBuilder;
use crate::monitor::{ActorMetaData, ChannelMetaData, SteadyTelemetryActorSend, SteadyTelemetrySend};
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry::setup;
use crate::util::steady_logging_init;// re-publish in public
use futures::FutureExt; // Provides the .fuse() method

// Re-exporting Bastion and run from bastion
pub use bastion::{Bastion, run};
pub use bastion::context::BastionContext;
pub use bastion::prelude::SupervisionStrategy;
pub use bastion::message::MessageHandler;
use futures::select;

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



#[derive(PartialEq, Eq, Debug, Clone)]
pub enum GraphLivelinessState {
    Building,
    Running,
    StopRequested, //All actors should finish immediate work, may be killed shortly
    StopVetoed, //Actor must log why they vetoed the stop but can return to StopRequested
                //only if there are no outstanding raised exceptions
                //if true and the timeout is also expired we will go to the Stopped state
    Stopped,
}

pub struct GraphLiveliness {
    pub(crate) state: GraphLivelinessState,
    pub(crate) objections: Arc<Vec<RaisedObjection>>,
    pub(crate) confirmations: Arc<Vec<ActorIdentity>>, //steadyContext
}
impl GraphLiveliness {
    pub fn new() -> Self {
        GraphLiveliness {
            state: GraphLivelinessState::Building,
            objections: Arc::new(Vec::new()),
            confirmations: Arc::new(Vec::new()),
        }
    }

}

pub struct RaisedObjection {
    actor_id: usize,
    actor_name: &'static str,
    reason: String
}

#[derive(Clone,Debug,Default,Copy)]
pub struct ActorIdentity {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
}

pub struct SteadyContext {
    pub(crate) ident: ActorIdentity,
    pub(crate) redundancy: usize,
    pub(crate) ctx: Option<BastionContext>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<Mutex<GraphLiveliness>>,
    pub(crate) count_restarts: Arc<AtomicU32>,
    pub(crate) args: Arc<Box<dyn Any+Send+Sync>>,
    pub(crate) actor_metadata: Arc<ActorMetaData>,
}

impl SteadyContext {

    pub fn liveliness(&self) -> Arc<Mutex<GraphLiveliness>> {
        self.runtime_state.clone()
    }

    pub fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    pub fn identity(&self) -> ActorIdentity {
        self.ident
    }

    pub fn ctx(&self) -> &Option<BastionContext> {
        &self.ctx
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

        let start_instant = Instant::now().sub(Duration::from_secs(1+config::TELEMETRY_PRODUCTION_RATE_MS as u64));


        // this is my fixed size version for this specific thread
        LocalMonitor::<RX_LEN, TX_LEN> {
            telemetry_send_rx,
            telemetry_send_tx,
            telemetry_state,
            last_telemetry_send: start_instant,

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
    pub(crate) all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<Mutex<GraphLiveliness>>,
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


impl Graph {

    pub fn actor_builder(&mut self) -> ActorBuilder{
        crate::ActorBuilder::new(self)
    }


    fn enable_fail_fast(&self) {
        std::panic::set_hook(Box::new(|panic_info| {
            let backtrace = Backtrace::capture();
            // You can log the panic information here if needed
            eprintln!("Application panicked: {}", panic_info);
            eprintln!("Backtrace:\n{:?}", backtrace);
            exit(-1);
        }));
    }

    pub fn start(&mut self) {

        // if we are not in release mode we will enable fail fast
        // this is most helpful while new code is under development
        if !config::DISABLE_DEBUG_FAIL_FAST {
            #[cfg(debug_assertions)]
            self.enable_fail_fast();
        }

        Bastion::start(); //start the graph
        let mut guard = run!(self.runtime_state.lock());
        let state = guard.deref_mut();
        state.state = GraphLivelinessState::Running;
    }


    pub fn stop(&mut self, clean_shutdown_timeout: Duration) {
        let now = Instant::now();
        //duration is not allowed to be less than 3 frames of telemetry
        //this ensures with safety that all actors had an opportunity to
        //raise objections and delay the stop. we just take the max of both
        //durations and do not error or panic since we are shutting down
        let timeout = clean_shutdown_timeout.max(
            Duration::from_millis(3*config::TELEMETRY_PRODUCTION_RATE_MS as u64));

        {
          let mut guard = run!(self.runtime_state.lock());
          let state = guard.deref_mut();
          state.state = GraphLivelinessState::StopRequested;
        }
        //wait for either the timeout or the state to be Stopped
        //while try lock then yield and do until time has passed
        //if we are stopped we will return immediately
        loop {
            //yield to other threads as we are trying to stop
            std::thread::yield_now();
            //allow bastion to process other work while we wait one frame
            run!(Delay::new(Duration::from_millis(config::TELEMETRY_PRODUCTION_RATE_MS as u64)));
            //now check the lock
            if let Some(mut lock) = self.runtime_state.try_lock() {
              if lock.state.eq(&GraphLivelinessState::Stopped) {
                  //TODO: how to make the eager? count down to stopped?
                  break;
              }
              if lock.state.eq(&GraphLivelinessState::StopRequested) {
                  assert!(lock.objections.is_empty(), "raised objections must be cleared before we can stop");
                  if now.elapsed() > timeout {
                      lock.state = GraphLivelinessState::Stopped; //release block
                      break;
                  }
              }
          }
        }
        Bastion::kill();
    }

    pub fn block_until_stopped(&mut self) {
        run!(async { //TODO: may also need to check for ctrl-c ??? check later


         //stay here until we see Stopped!!


          //  self.runtime_state.lock

          //  GraphLivelinessState::Stopped

            //Delay::new(Duration::from_secs(opt.duration)).await;
            //graph.stop(Duration::from_secs(2));
        });

    }



    /// create a new graph for the application typically done in main
    pub fn new<A: Any+Send+Sync>(args: A) -> Graph {
        Graph {
            args: Arc::new(Box::new(args)),
            channel_count: Arc::new(AtomicUsize::new(0)),
            monitor_count: Arc::new(AtomicUsize::new(0)), //this is the count of all monitors
            all_telemetry_rx: Arc::new(Mutex::new(Vec::new())), //this is all telemetry receivers
            runtime_state: Arc::new(Mutex::new(GraphLiveliness::new())),
        }
    }

    pub fn channel_builder(&mut self) -> ChannelBuilder {
        ChannelBuilder::new(self.channel_count.clone())
    }

    pub fn init_telemetry(&mut self) {
        telemetry::setup::build_optional_telemetry_graph(self);
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
    pub(crate) id: usize,
    pub(crate) tx: InternalSender<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize,
    pub(crate) last_error_send: Instant,
    pub(crate) is_closed: Arc<AtomicBool>, //only on shutdown
}

pub struct Rx<T> {
    pub(crate) id: usize,
    pub(crate) rx: InternalReceiver<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize,
    pub(crate) is_closed: Arc<AtomicBool>, //only on shutdown
}

////////////////////////////////////////////////////////////////
impl<T> Tx<T> {

    ///the Rx should not expect any more messages than those already found
    ///on the channel. This is a signal that this actor has probably stopped
    pub fn mark_closed(&mut self) {
        self.is_closed.store(true, Ordering::SeqCst);
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
    pub async fn send_async(& mut self, msg: T) -> Result<(), T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_send_async(msg).await
    }

    //////////////////////////////////////////////////////////////////
    //////////////////////////////////////////////////////////////////

    fn direct_use_check_and_warn(&self) {
        if self.channel_meta_data.expects_to_be_monitored {
            warn!("you called this without the monitor but monitoring for this channel is enabled. see the monitor version of this method");
            //print stacktrace
            let stack = backtrace::Backtrace::new();
            error!("stack: {:?}", stack);
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
    async fn shared_send_async(& mut self, msg: T) -> Result<(), T> {
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())},
            Err(msg) => {
                //NOTE: we slow the rate of errors reported
                if self.last_error_send.elapsed().as_secs() > 10 {
                       let type_name = type_name::<T>().split("::").last();

                       error!("tx full channel #{} {:?} cap:{:?} type:{:?} "
                                   , self.id
                                   , self.channel_meta_data.labels
                                   , self.tx.capacity()
                                   , type_name);
                       self.last_error_send = Instant::now();
                }
                // here we will wait until there is room in the channel

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




impl<T> Rx<T> {

    pub fn is_closed(&self) {
        self.is_closed.load(Ordering::SeqCst);
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
    pub async fn wait_avail_units(& mut self, count: usize) {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_wait_avail_units(count).await
    }
    //////////////////////////////////////////////////////////////////
    //////////////////////////////////////////////////////////////////

    fn direct_use_check_and_warn(&self) {
        if self.channel_meta_data.expects_to_be_monitored {
            warn!("you called this without the monitor but monitoring for this channel is enabled. see the monitor version of this method");
            //print stacktrace
            let stack = backtrace::Backtrace::new();
            error!("stack: {:?}", stack);
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
pub trait RxDef: Debug {
    fn meta_data(&self) -> Arc<ChannelMetaData>;
}

impl <T> TxDef for SteadyTx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        run!( async {
                let guard = self.lock().await;
                let this = guard.deref();
                this.channel_meta_data.clone()
            })
    }
}

impl <T> RxDef for SteadyRx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        run!( async {
                let guard = self.lock().await;
                let this = guard.deref();
                this.channel_meta_data.clone()
            })
    }}


pub struct SteadyBundle{}

impl SteadyBundle {
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


    pub fn rx_def_slice< T, const GIRTH: usize>(this: & SteadyRxBundle<T, GIRTH>) -> [& dyn RxDef; GIRTH] {
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

impl <const RXL: usize, const TXL: usize> Drop for LocalMonitor<RXL, TXL> {
    //if possible we never want to loose telemetry data so we try to flush it out
    fn drop(&mut self) {
        run!(
            telemetry::setup::send_all_local_telemetry_async(self)
          );
    }

}

pub struct LocalMonitor<const RX_LEN: usize, const TX_LEN: usize> {
    pub(crate) ident:                ActorIdentity,
    pub(crate) ctx:                  Option<BastionContext>,
    pub(crate) telemetry_send_tx:    Option<SteadyTelemetrySend<TX_LEN>>,
    pub(crate) telemetry_send_rx:    Option<SteadyTelemetrySend<RX_LEN>>,
    pub(crate) telemetry_state:      Option<SteadyTelemetryActorSend>,
    pub(crate) last_telemetry_send:  Instant,
    pub(crate) runtime_state:        Arc<Mutex<GraphLiveliness>>,
    pub(crate) redundancy:           usize,


    #[cfg(test)]
    pub(crate) test_count: HashMap<&'static str, usize>,

}

///////////////////
impl <const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {


    /// Returns the unique identifier of the LocalMonitor instance.
    ///
    /// # Returns
    /// A `usize` representing the monitor's unique ID.
    pub fn id(&self) -> usize {
        self.ident.id
    }

    /// Retrieves the static name assigned to the LocalMonitor instance.
    ///
    /// # Returns
    /// A static string slice (`&'static str`) representing the monitor's name.
    pub fn name(&self) -> & 'static str {
        self.ident.name
    }

    /// Indicates the level of redundancy applied to the monitoring process.
    ///
    /// # Returns
    /// A `usize` value representing the redundancy level.
    pub fn redundancy(&self) -> usize {
        self.redundancy
    }

    /// Initiates a stop signal for the LocalMonitor, halting its monitoring activities.
    ///
    /// # Returns
    /// A `Result` indicating successful cessation of monitoring activities.
    pub async fn stop(&mut self) -> Result<(),()>  {
        if let Some(ref mut st) = self.telemetry_state {
            st.bool_stop = true;
        }// upon drop we will flush telemetry
        Ok(())
    }


    pub fn liveliness(&self) -> Arc<Mutex<GraphLiveliness>> {
        self.runtime_state.clone()
    }

    /// Retrieves an optional reference to the `BastionContext` associated with the LocalMonitor.
    ///
    /// # Returns
    /// An `Option` containing a reference to the `BastionContext` if available.
    pub fn ctx(&self) -> Option<&BastionContext> {
        if let Some(ctx) = &self.ctx {
            Some(ctx)
        } else {
            None
        }
    }


    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    ///
    /// # Asynchronous
    pub async fn relay_stats_all(&mut self) {
        //NOTE: not time ing this one as it is mine and internal
        telemetry::setup::try_send_all_local_telemetry(self).await;
    }

    /// Periodically relays telemetry data at a specified rate.
    ///
    /// # Parameters
    /// - `duration_rate`: The interval at which telemetry data should be sent.
    ///
    /// # Asynchronous
    pub async fn relay_stats_periodic(&mut self, duration_rate: Duration) {
        self.start_hot_profile(monitor::CALL_WAIT);
        assert!(duration_rate.ge(&Duration::from_micros(config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u64)),
              "the set rate is too fast, it must be at least {} micro seconds but found {:?}", config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS,duration_rate);

        Delay::new(duration_rate.saturating_sub(self.last_telemetry_send.elapsed())).await;
        self.rollup_hot_profile();
        //this can not be measured since it sends the measurement of hot_profile.
        //also this is a special case where we do not want to measure the time it takes to send telemetry
        self.relay_stats_all().await;
    }

    /// Marks the start of a high-activity profile period for telemetry monitoring.
    ///
    /// # Parameters
    /// - `x`: The index representing the type of call being monitored.
    fn start_hot_profile(&mut self, x: usize) {
        if let Some(ref mut st) = self.telemetry_state {
            st.calls[x] = st.calls[x].saturating_add(1);
            if st.hot_profile.is_none() {
                st.hot_profile = Some(Instant::now())
            }
        };
    }

    /// Finalizes the current hot profile period, aggregating the collected telemetry data.
    fn rollup_hot_profile(&mut self) {
        if let Some(ref mut st) = self.telemetry_state {
            if let Some(d) = st.hot_profile.take() {
                st.await_ns_unit += Instant::elapsed(&d).as_nanos() as u64;
                assert!(st.instant_start.le(&d), "unit_start: {:?} call_start: {:?}", st.instant_start, d);
            }
        }
    }

    /// Attempts to peek at a slice of messages without removing them from the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance for peeking.
    /// - `elems`: A mutable slice to store the peeked messages.
    ///
    /// # Returns
    /// The number of messages peeked and stored in `elems`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    pub fn try_peek_slice<T>(& mut self, this: &mut Rx<T>, elems: &mut [T]) -> usize
        where T: Copy {
        this.shared_try_peek_slice(elems)
    }

    /// Asynchronously peeks at a slice of messages, waiting for a specified count to be available.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `wait_for_count`: The number of messages to wait for before peeking.
    /// - `elems`: A mutable slice to store the peeked messages.
    ///
    /// # Returns
    /// The number of messages peeked and stored in `elems`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    ///
    /// # Asynchronous
    pub async fn peek_async_slice<T>(&mut self, this: &mut Rx<T>, wait_for_count: usize, elems: &mut [T]) -> usize
    where T: Copy {
        this.shared_peek_async_slice(wait_for_count,elems).await
    }

    /// Retrieves and removes a slice of messages from the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `slice`: A mutable slice where the taken messages will be stored.
    ///
    /// # Returns
    /// The number of messages actually taken and stored in `slice`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    pub fn take_slice<T>(&mut self, this: & mut Rx<T>, slice: &mut [T]) -> usize
    where T: Copy {
        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_BATCH_READ] = st.calls[monitor::CALL_BATCH_READ].saturating_add(1);
        }
        let done = this.shared_take_slice(slice);
        this.local_index = if let Some(ref mut tel)= self.telemetry_send_rx {
            tel.process_event(this.local_index, this.id, done)
        } else {
            MONITOR_NOT
        };
        done
    }

    /// Attempts to take a single message from the channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` if a message is available, or `None` if the channel is empty.
    pub fn try_take<T>(&mut self, this: & mut Rx<T>) -> Option<T> {
        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_SINGLE_READ]=st.calls[monitor::CALL_SINGLE_READ].saturating_add(1);
        }
        match this.shared_try_take() {
            Some(msg) => {
                this.local_index = if let Some(ref mut tel)= self.telemetry_send_rx {
                    tel.process_event(this.local_index, this.id, 1)
                } else {
                    MONITOR_NOT
                };
                Some(msg)
            },
            None => {None}
        }
    }

    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message is available, or `None` if the channel is empty.
    pub fn try_peek<'a,T>(&'a mut self, this: &'a mut Rx<T>) -> Option<&T> {
        this.shared_try_peek()
    }

    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    pub fn try_peek_iter<'a,T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a {
        this.shared_try_peek_iter()
    }

    /// Asynchronously returns an iterator over the messages in the channel,
    /// waiting for a specified number of messages to be available.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `wait_for_count`: The number of messages to wait for before returning the iterator.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Asynchronous
    pub async fn peek_async_iter<'a,T>(&'a mut self, this: &'a mut Rx<T>, wait_for_count: usize) -> impl Iterator<Item = &'a T> + 'a {
        self.start_hot_profile(monitor::CALL_OTHER);
        let result = this.shared_peek_async_iter(wait_for_count).await;
        self.rollup_hot_profile();
        result
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    pub fn is_empty<T>(& mut self, this: & mut Rx<T>) -> bool {
        this.shared_is_empty()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    pub fn avail_units<T>(& mut self, this: & mut Rx<T>) -> usize {
        this.shared_avail_units()
    }

    /// Asynchronously waits for a specified duration.
    ///
    /// # Parameters
    /// - `duration`: The duration to wait.
    ///
    /// # Asynchronous
    pub async fn wait(& mut self, duration: Duration) {
        self.start_hot_profile(monitor::CALL_WAIT);
        Delay::new(duration).await;
        self.rollup_hot_profile();
    }

    /// Calls an asynchronous function and monitors its execution for telemetry.
    ///
    /// # Parameters
    /// - `f`: The asynchronous function to call.
    ///
    /// # Returns
    /// The output of the asynchronous function `f`.
    ///
    /// # Asynchronous
    pub async fn call_async<F>(&mut self, f: F) -> F::Output
        where F: Future {
        self.start_hot_profile(monitor::CALL_OTHER);
        let result = f.await;
        self.rollup_hot_profile();
        result
    }

    /// Asynchronously waits until a specified number of units are available in the Rx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `count`: The number of units to wait for availability.
    ///
    /// # Asynchronous
    pub async fn wait_avail_units<T>(&mut self, this: & mut Rx<T>, count:usize) {
        self.start_hot_profile(monitor::CALL_OTHER);
        let result = this.shared_wait_avail_units(count).await;
        self.rollup_hot_profile();
        result
    }

    /// Asynchronously peeks at the next available message in the channel without removing it.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message becomes available, or `None` if the channel is closed.
    ///
    /// # Asynchronous
    pub async fn peek_async<'a,T>(&'a mut self, this: &'a mut Rx<T>) -> Option<&T> {
        self.start_hot_profile(monitor::CALL_OTHER);
        let result = this.shared_peek_async().await;
        self.rollup_hot_profile();
        result
    }

    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `Result<T, String>`, where `Ok(T)` is the message if available, and `Err(String)` contains an error message if the retrieval fails.
    ///
    /// # Asynchronous
    pub async fn take_async<T>(& mut self, this: & mut Rx<T>) -> Result<T, String> {
        self.start_hot_profile(monitor::CALL_SINGLE_READ);
        let result = this.shared_take_async().await;
        self.rollup_hot_profile();
        match result {
            Ok(result) => {
                this.local_index = if let Some(ref mut tel)= self.telemetry_send_rx {
                    tel.process_event(this.local_index, this.id, 1)
                } else {
                    MONITOR_NOT
                };
                #[cfg(test)]
                self.test_count.entry("take_async").and_modify(|e| *e += 1).or_insert(1);
                Ok(result)
            },
            Err(error_msg) => {
                error!("Unexpected error take_async: {} {}", error_msg, self.ident.name);
                Err(error_msg)
            }
        }
    }

    /// Sends a slice of messages to the Tx channel until it is full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `slice`: A slice of messages to be sent.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    pub fn send_slice_until_full<T>(&mut self, this: & mut Tx<T>, slice: &[T]) -> usize
        where T: Copy {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_BATCH_WRITE]=st.calls[monitor::CALL_BATCH_WRITE].saturating_add(1);
        }

        let done = this.send_slice_until_full(slice);

        this.local_index = if let Some(ref mut tel)= self.telemetry_send_tx {
            tel.process_event(this.local_index, this.id, done)
        } else {
            MONITOR_NOT
        };

        done
    }

    /// Sends messages from an iterator to the Tx channel until it is full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `iter`: An iterator that yields messages of type `T`.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    pub fn send_iter_until_full<T,I: Iterator<Item = T>>(&mut self, this: & mut Tx<T>, iter: I) -> usize {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_BATCH_WRITE]=st.calls[monitor::CALL_BATCH_WRITE].saturating_add(1);
        }

        let done = this.send_iter_until_full(iter);

        this.local_index = if let Some(ref mut tel)= self.telemetry_send_tx {
            tel.process_event(this.local_index, this.id, done)
        } else {
            MONITOR_NOT
        };

        done
    }

    /// Attempts to send a single message to the Tx channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates successful send and `Err(T)` returns the message if the channel is full.
    pub fn try_send<T>(& mut self, this: & mut Tx<T>, msg: T) -> Result<(), T> {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_SINGLE_WRITE]=st.calls[monitor::CALL_SINGLE_WRITE].saturating_add(1);
        }

        match this.try_send(msg) {
            Ok(_) => {

                this.local_index = if let Some(ref mut tel)= self.telemetry_send_tx {
                    tel.process_event(this.local_index, this.id, 1)
                } else {
                    MONITOR_NOT
                };
                Ok(())
            },
            Err(sensitive) => {
                error!("Unexpected error try_send  telemetry: {} type: {}"
                    , self.ident.name, type_name::<T>());
                Err(sensitive)
            }
        }
    }

    /// Checks if the Tx channel is currently full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    pub fn is_full<T>(& mut self, this: & mut Tx<T>) -> bool {
        this.is_full()
    }

    /// Returns the number of vacant units in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    pub fn vacant_units<T>(& mut self, this: & mut Tx<T>) -> usize {
        this.shared_vacant_units()
    }

    /// Asynchronously waits until at least a specified number of units are vacant in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Asynchronous
    pub async fn wait_vacant_units<T>(& mut self, this: & mut Tx<T>, count:usize) {
        self.start_hot_profile(monitor::CALL_WAIT);
        let response = this.shared_wait_vacant_units(count).await;
        self.rollup_hot_profile();
        response
    }

    /// Asynchronously waits until the Tx channel is empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Asynchronous
    pub async fn wait_empty<T>(& mut self, this: & mut Tx<T>) {
        self.start_hot_profile(monitor::CALL_WAIT);
        let response = this.shared_wait_empty().await;
        self.rollup_hot_profile();
        response
    }

    /// Sends a message to the Tx channel asynchronously, waiting if necessary until space is available.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `a`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates that the message was successfully sent, and `Err(T)` if the send operation could not be completed.
    ///
    /// # Asynchronous
    pub async fn send_async<T>(& mut self, this: & mut Tx<T>, a: T) -> Result<(), T> {
       self.start_hot_profile(monitor::CALL_SINGLE_WRITE);
       let result = this.shared_send_async(a).await;
       self.rollup_hot_profile();
       match result  {
           Ok(_) => {
               this.local_index = if let Some(ref mut tel)= self.telemetry_send_tx {
                   tel.process_event(this.local_index, this.id, 1)
               } else {
                   MONITOR_NOT
               };
               Ok(())
           },
           Err(sensitive) => {
               error!("Unexpected error send_async telemetry: {} type: {}", self.ident.name, type_name::<T>());
               Err(sensitive)
           }
       }
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Percentile(f64);

impl Percentile {
    // Private constructor to directly set the value inside the struct.
    // Ensures that all public constructors go through validation.
    fn new(value: f64) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    // Convenience methods for common percentiles
    pub fn p25() -> Self {
        Self(25.0)
    }

    pub fn p50() -> Self {Self(50.0) }

    pub fn p75() -> Self {
        Self(75.0)
    }

    pub fn p90() -> Self {
        Self(90.0)
    }

    pub fn p80() -> Self {
        Self(80.0)
    }

    pub fn p96() -> Self {
        Self(96.0)
    }

    pub fn p99() -> Self {
        Self(99.0)
    }

    // Allows custom values within the valid range.
    pub fn custom(value: f64) -> Option<Self> {
        Self::new(value)
    }

    // Getter to access the inner f32 value.
    pub fn percentile(&self) -> f64 {
        self.0
    }
}

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

impl Metric for MCPU {}
impl Metric for Work {}

impl Metric for Rate {}

impl Metric for Filled {}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MCPU {
    mcpu: u16, // max 1024
}

impl MCPU {
    pub fn new(value: u16) -> Option<Self> {
        if value<=1024 {
            Some(Self {
                mcpu: value,
            })
        } else {
            None
        }
    }

    pub fn rational(&self) -> (u64,u64) {
        (self.mcpu as u64,1024)
    }

    pub fn m16() -> Self { MCPU{mcpu:16}}
    pub fn m64() -> Self { MCPU{mcpu:64}}
    pub fn m256() -> Self { MCPU{mcpu:256}}
    pub fn m512() -> Self { MCPU{mcpu:512}}
    pub fn m768() -> Self { MCPU{mcpu:768}}
    pub fn m1024() -> Self { MCPU{mcpu:1024}}

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    work: u16, // out of 10000 where 10000 is 100%
}

impl Work {
    pub fn new(value: f32) -> Option<Self> {
        if (0.0..=100.00).contains(&value) {
            Some(Work{work: (value * 100.0) as u16}) //10_000 is 100%
        } else {
            None
        }
    }

    pub fn rational(&self) -> (u64,u64) {
        (self.work as u64, 10_000)
    }

    pub fn p10() -> Self { Work{ work: 1000 }}
    pub fn p20() -> Self { Work{ work: 2000 }}
    pub fn p30() -> Self { Work{ work: 3000 }}
    pub fn p40() -> Self { Work{ work: 4000 }}
    pub fn p50() -> Self { Work{ work: 5000 }}
    pub fn p60() -> Self { Work{ work: 6000 }}
    pub fn p70() -> Self { Work{ work: 7000 }}
    pub fn p80() -> Self { Work{ work: 8000 }}
    pub fn p90() -> Self { Work{ work: 9000 }}
    pub fn p100() -> Self { Work{ work: 10000 }}

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    // Internal representation as a rational number of the rate per ms
    // Numerator: units, Denominator: time in ms
    numerator: u64,
    denominator: u64,
}

impl Rate {
    // Milliseconds
    pub fn per_millis(units: u64) -> Self {
        Self {
            numerator: units,
            denominator: 1,
        }
    }

    pub fn per_seconds(units: u64) -> Self {
        Self {
            numerator: units * 1000,
            denominator: 1,
        }
    }

    pub fn per_minutes(units: u64) -> Self {
        Self {
            numerator: units * 1000 * 60,
            denominator:  1,
        }
    }

    pub fn per_hours(units: u64) -> Self {
        Self {
            numerator: units * 1000 * 60 * 60,
            denominator:  1,
        }
    }

    pub fn per_days(units: u64) -> Self {
        Self {
            numerator: units * 1000 * 60 * 60 * 24,
            denominator: 1,
        }
    }

    /// Returns the rate as a rational number (numerator, denominator) to represent the rate per ms.
    /// This method ensures the rate can be used without performing division, preserving precision.
    pub(crate) fn rational_ms(&self) -> (u64, u64) {
        (self.numerator, self.denominator)
    }
}

///////////////////////////////


#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filled {
    Percentage(u64,u64),  // Represents a percentage filled, as numerator and denominator
    Exact(u64),           // Represents an exact fill level
}

impl Filled {
    /// Creates a new `Filled` instance representing a percentage filled.
    /// Ensures the percentage is within the valid range of 0.0 to 100.0.
    pub fn percentage(value: f32) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self::Percentage((value * 100_000f32) as u64, 100_000u64))
        } else {
            None
        }
    }

    pub fn p10() -> Self { Self::Percentage(10, 100)}
    pub fn p20() -> Self { Self::Percentage(20, 100)}
    pub fn p30() -> Self { Self::Percentage(30, 100)}
    pub fn p40() -> Self { Self::Percentage(40, 100)}
    pub fn p50() -> Self { Self::Percentage(50, 100)}
    pub fn p60() -> Self { Self::Percentage(60, 100)}
    pub fn p70() -> Self { Self::Percentage(70, 100)}
    pub fn p80() -> Self { Self::Percentage(80, 100)}
    pub fn p90() -> Self { Self::Percentage(90, 100)}
    pub fn p100() -> Self { Self::Percentage(100, 100)}


    /// Creates a new `Filled` instance representing an exact fill level.
    pub fn exact(value: u64) -> Self {
        Self::Exact(value)
    }
}

