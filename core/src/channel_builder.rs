use std::fmt::Debug;
use ringbuf::storage::Heap;
use std::sync::Arc;
use futures::lock::Mutex;
use std::time::{Duration, Instant};
use async_ringbuf::AsyncRb;
use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicUsize, Ordering};
use async_ringbuf::producer::AsyncProducer;

pub(crate) type ChannelBacking<T> = Heap<T>;
pub(crate) type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;
pub(crate) type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;



//TODO: 2025, we should use static for all telemetry work.
//      this might lead to a general solution for static in other places
//let mut rb = AsyncRb::<Static<T, 12>>::default().split_ref()
//   AsyncRb::<ChannelBacking<T>>::new(cap).split_ref()

//# use ringbuf::traits::Split;
use async_ringbuf::wrap::{AsyncCons, AsyncProd};

use futures::channel::oneshot;
#[allow(unused_imports)]
use log::*;
use async_ringbuf::traits::Split;
use futures_timer::Delay;
use ringbuf::traits::Observer;
use crate::{abstract_executor, AlertColor, LazySteadyRxBundle, LazySteadyTxBundle, Metric, MONITOR_UNKNOWN, StdDev, SteadyRx, SteadyRxBundle, SteadyTx, SteadyTxBundle, Trigger};
use crate::actor_builder::{ActorBuilder, Percentile};
use crate::monitor::ChannelMetaData;
use crate::steady_rx::{Rx};
use crate::steady_tx::{Tx};

/// Builder for configuring and creating channels within the Steady State framework.
///
/// The `ChannelBuilder` struct allows for the detailed configuration of channels, including
/// performance metrics, thresholds, and other operational parameters. This provides flexibility
/// in tailoring channels to specific requirements and monitoring needs.
#[derive(Clone, Debug)]
pub struct ChannelBuilder {
    /// The noise threshold as an `Instant`.
    ///
    /// This field is used to manage and mitigate noise in the channel's operation.
    noise_threshold: Instant,

    /// The count of channels, stored as an `Arc<AtomicUsize>`.
    ///
    /// This field tracks the number of channels created.
    channel_count: Arc<AtomicUsize>,

    /// The capacity of the channel.
    ///
    /// This field defines the maximum number of messages the channel can hold.
    capacity: usize,

    /// Labels associated with the channel.
    ///
    /// This field holds a static reference to an array of static string slices, used for identifying the channel.
    labels: &'static [&'static str],

    /// Indicates whether the labels should be displayed.
    ///
    /// If `true`, the labels will be shown in monitoring outputs.
    display_labels: bool,

    /// The refresh rate for monitoring data, expressed in bits.
    ///
    /// This field defines how frequently the monitoring data should be refreshed.
    refresh_rate_in_bits: u8,

    /// The size of the window bucket for metrics, expressed in bits.
    ///
    /// This field defines the size of the window bucket used for metrics aggregation, ensuring it is a power of 2.
    window_bucket_in_bits: u8,

    /// if not NAN Indicates line expansion enabled rate
    ///
    line_expansion: f32,

    /// Indicates whether the type of the channel should be displayed.
    ///
    /// If `true`, the type information will be included in monitoring outputs.
    show_type: bool,

    /// A list of percentiles for the filled capacity of the channel.
    ///
    /// Each element represents a row in the monitoring output.
    percentiles_filled: Vec<Percentile>,

    /// A list of percentiles for the rate of messages in the channel.
    ///
    /// Each element represents a row in the monitoring output.
    percentiles_rate: Vec<Percentile>,

    /// A list of percentiles for the latency of messages in the channel.
    ///
    /// Each element represents a row in the monitoring output.
    percentiles_latency: Vec<Percentile>,

    /// A list of standard deviations for the filled capacity of the channel.
    ///
    /// Each element represents a row in the monitoring output.
    std_dev_filled: Vec<StdDev>,

    /// A list of standard deviations for the rate of messages in the channel.
    ///
    /// Each element represents a row in the monitoring output.
    std_dev_rate: Vec<StdDev>,

    /// A list of standard deviations for the latency of messages in the channel.
    ///
    /// Each element represents a row in the monitoring output.
    std_dev_latency: Vec<StdDev>,

    /// A list of triggers for the rate of messages with associated alert colors.
    ///
    /// Each tuple contains a trigger condition and an alert color, with the base color being green if used.
    trigger_rate: Vec<(Trigger<Rate>, AlertColor)>,

    /// A list of triggers for the filled capacity with associated alert colors.
    ///
    /// Each tuple contains a trigger condition and an alert color, with the base color being green if used.
    trigger_filled: Vec<(Trigger<Filled>, AlertColor)>,

    /// A list of triggers for the latency of messages with associated alert colors.
    ///
    /// Each tuple contains a trigger condition and an alert color, with the base color being green if used.
    trigger_latency: Vec<(Trigger<Duration>, AlertColor)>,

    /// Indicates whether the average rate of messages should be monitored.
    ///
    /// If `true`, the average rate is tracked for this channel.
    avg_rate: bool,

    /// Indicates whether the average filled capacity should be monitored.
    ///
    /// If `true`, the average filled capacity is tracked for this channel.
    avg_filled: bool,

    /// Indicates whether the average latency should be monitored.
    ///
    /// If `true`, the average latency is tracked for this channel.
    avg_latency: bool,

    /// Indicates whether the channel connects to a sidecar.
    ///
    /// If `true`, the channel is connected to a sidecar for additional processing or monitoring.
    connects_sidecar: bool,

    /// Indicates whether the maximum filled capacity should be monitored.
    ///
    /// If `true`, the maximum filled capacity is tracked for this channel.
    max_filled: bool,

    /// Indicates whether the minimum filled capacity should be monitored.
    ///
    /// If `true`, the minimum filled capacity is tracked for this channel.
    min_filled: bool,

    /// The frame rate for monitoring updates, expressed in milliseconds.
    ///
    /// This field defines the interval at which monitoring data is updated.
    frame_rate_ms: u64,

    /// A vector of one-shot shutdown senders.
    ///
    /// This field is used to manage shutdown signals for the channel, wrapped in an `Arc<Mutex<>>` for thread safety.
    oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
}


const DEFAULT_CAPACITY: usize = 64;
impl ChannelBuilder {
    /// Creates a new `ChannelBuilder` instance with the specified parameters.
    ///
    /// # Parameters
    /// - `channel_count`: Shared counter for the number of channels.
    /// - `oneshot_shutdown_vec`: Shared vector of one-shot shutdown senders.
    /// - `noise_threshold`: Instant used as a noise threshold.
    /// - `frame_rate_ms`: Frame rate in milliseconds.
    ///
    /// # Returns
    /// A new instance of `ChannelBuilder` with default values.
    pub(crate) fn new(
        channel_count: Arc<AtomicUsize>,
        oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
        noise_threshold: Instant,
        frame_rate_ms: u64,
    ) -> ChannelBuilder {

        //build default window
        let (refresh_in_bits, window_in_bits) = ActorBuilder::internal_compute_refresh_window(frame_rate_ms as u128
                                                                                              , Duration::from_secs(1)
                                                                                              , Duration::from_secs(10));        
        ChannelBuilder {
            noise_threshold,
            channel_count,
            capacity: DEFAULT_CAPACITY,
            labels: &[],
            display_labels: false,
            oneshot_shutdown_vec,
            refresh_rate_in_bits: refresh_in_bits, 
            window_bucket_in_bits: window_in_bits, 
            line_expansion: f32::NAN, // use 1.0 as the default
            show_type: false,
            percentiles_filled: Vec::with_capacity(0),
            percentiles_rate: Vec::with_capacity(0),
            percentiles_latency: Vec::with_capacity(0),
            std_dev_filled: Vec::with_capacity(0),
            std_dev_rate: Vec::with_capacity(0),
            std_dev_latency: Vec::with_capacity(0),
            trigger_rate: Vec::with_capacity(0),
            trigger_filled: Vec::with_capacity(0),
            trigger_latency: Vec::with_capacity(0),
            avg_filled: false,
            avg_rate: false,
            avg_latency: false,
            max_filled: false,
            min_filled: false,
            connects_sidecar: false,
            frame_rate_ms,
        }
    }

    /// Computes and sets the refresh rate and window size for the channel.
    ///
    /// # Parameters
    /// - `refresh`: Duration of the refresh rate.
    /// - `window`: Duration of the window size.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with computed refresh rate and window size.
    pub fn with_compute_refresh_window_floor(&self, refresh: Duration, window: Duration) -> Self {
        let mut result = self.clone(); 
        let (refresh_in_bits, window_in_bits) = ActorBuilder::internal_compute_refresh_window(self.frame_rate_ms as u128, refresh, window);
        result.refresh_rate_in_bits = refresh_in_bits;
        result.window_bucket_in_bits = window_in_bits;        
        result
    }

    /// Disables any metric collection
    ///
    pub fn with_no_refresh_window(&self) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = 0;
        result.window_bucket_in_bits = 0;
        result
    }


    /// Builds a bundle of channels with the specified girth.
    ///
    /// # Parameters
    /// - `GIRTH`: The number of channels in the bundle.
    ///
    /// # Returns
    /// A tuple containing bundles of transmitters and receivers.
    pub fn build_as_eager_bundle<T, const GIRTH: usize>(&self) -> (SteadyTxBundle<T, GIRTH>, SteadyRxBundle<T, GIRTH>) {
        let mut tx_vec = Vec::with_capacity(GIRTH);
        let mut rx_vec = Vec::with_capacity(GIRTH);

        (0..GIRTH).for_each(|_| {
            let (t, r) = self.eager_build();
            tx_vec.push(t);
            rx_vec.push(r);
        });

        (
            Arc::new(tx_vec.try_into().expect("Incorrect length")),
            Arc::new(rx_vec.try_into().expect("Incorrect length")),
        )
    }

    /// Build as a bundle fo channels. Consumes the builder. Makes use of Lazy version which will
    /// postpone allocation until after the first clone() is called.
    ///
    /// # Parameters
    /// - `GIRTH`: The number of channels in the bundle.
    ///
    /// # Returns
    /// A tuple containing bundles of transmitters and receivers.
    pub fn build_as_bundle<T, const GIRTH: usize>(&self) -> (LazySteadyTxBundle<T, GIRTH>, LazySteadyRxBundle<T, GIRTH>) {
        let mut tx_vec = Vec::with_capacity(GIRTH);
        let mut rx_vec = Vec::with_capacity(GIRTH);

        (0..GIRTH).for_each(|_| {
            let (t, r) = self.build();
            tx_vec.push(t);
            rx_vec.push(r);
        });

        (
            {
                match tx_vec.try_into() {
                    Ok(t) => t,
                    Err(_) => panic!("Internal error, incorrect length")
                }
            },
            {
                match rx_vec.try_into() {
                    Ok(t) => t,
                    Err(_) => panic!("Internal error, incorrect length")
                }
            }
            ,
        )
    }


    /// Sets the capacity for the channel being built.
    ///
    /// # Parameters
    /// - `capacity`: The maximum number of messages the channel can hold.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with the specified capacity.
    pub fn with_capacity(&self, capacity: usize) -> Self {
        let mut result = self.clone();
        result.capacity = capacity;
        result
    }

    /// Enables type display in telemetry.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with type display enabled.
    pub fn with_type(&self) -> Self {
        let mut result = self.clone();
        result.show_type = true;
        result
    }

    /// Enables line expansion in telemetry visualization.
    ///
    /// # Parameters
    /// - `scale`: use 1.0 as the default scale factor
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with line expansion enabled.
    pub fn with_line_expansion(&self, scale: f32) -> Self {
        let mut result = self.clone();
        result.line_expansion = scale;
        result
    }

    /// Enables average calculation for the filled state of channels.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with average filled calculation enabled.
    pub fn with_avg_filled(&self) -> Self {
        let mut result = self.clone();
        result.avg_filled = true;
        result
    }

    /// Enables maximum filled state calculation for the channel.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with maximum filled calculation enabled.
    pub fn with_filled_max(&self) -> Self {
        let mut result = self.clone();
        result.max_filled = true;
        result
    }

    /// Enables minimum filled state calculation for the channel.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with minimum filled calculation enabled.
    pub fn with_filled_min(&self) -> Self {
        let mut result = self.clone();
        result.min_filled = true;
        result
    }

    /// Enables average rate calculation.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with average rate calculation enabled.
    pub fn with_avg_rate(&self) -> Self {
        let mut result = self.clone();
        result.avg_rate = true;
        result
    }

    /// Enables average latency calculation.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with average latency calculation enabled.
    pub fn with_avg_latency(&self) -> Self {
        let mut result = self.clone();
        result.avg_latency = true;
        result
    }

    /// Marks this connection as going to a sidecar, for display purposes.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance configured with this mark.
    pub fn connects_sidecar(&self) -> Self {
        let mut result = self.clone();
        result.connects_sidecar = true;
        result
    }

    /// Sets labels for the channel, optionally displaying them in telemetry.
    ///
    /// # Parameters
    /// - `labels`: Static slice of label strings.
    /// - `display`: Boolean indicating whether to display labels in telemetry.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with specified labels.
    pub fn with_labels(&self, labels: &'static [&'static str], display: bool) -> Self {
        let mut result = self.clone();
        result.labels = if display { labels } else { &[] };
        result
    }

    /// Configures the channel to calculate the standard deviation of the "filled" state.
    ///
    /// # Parameters
    /// - `config`: Configuration for standard deviation calculation.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with filled state standard deviation calculation enabled.
    pub fn with_filled_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_filled.push(config);
        result
    }

    /// Configures the channel to calculate the standard deviation of message rate.
    ///
    /// # Parameters
    /// - `config`: Configuration for standard deviation calculation of message rate.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with message rate standard deviation calculation enabled.
    pub fn with_rate_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_rate.push(config);
        result
    }

    /// Configures the channel to calculate the standard deviation of message latency.
    ///
    /// # Parameters
    /// - `config`: Configuration for standard deviation calculation of latency.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with latency standard deviation calculation enabled.
    pub fn with_latency_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_latency.push(config);
        result
    }

    /// Configures the channel to calculate specific percentiles for the "filled" state.
    ///
    /// # Parameters
    /// - `config`: Configuration for the percentile calculation of the filled state.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with filled state percentile calculation enabled.
    pub fn with_filled_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_filled.push(config);
        result
    }

    /// Configures the channel to calculate specific percentiles for message rate.
    ///
    /// # Parameters
    /// - `config`: Configuration for the percentile calculation of message rate.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with rate percentile calculation enabled.
    pub fn with_rate_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_rate.push(config);
        result
    }

    /// Configures the channel to calculate specific percentiles for message latency.
    ///
    /// # Parameters
    /// - `config`: Configuration for the percentile calculation of latency.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with latency percentile calculation enabled.
    pub fn with_latency_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_latency.push(config);
        result
    }

    /// Configures triggers based on message rate with associated alert colors.
    ///
    /// # Parameters
    /// - `bound`: The threshold for the trigger.
    /// - `color`: The color to represent the alert when the threshold is crossed.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with a rate trigger configured.
    pub fn with_rate_trigger(&self, bound: Trigger<Rate>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_rate.push((bound, color));
        result
    }

    /// Configures triggers based on the "filled" state of the channel with associated alert colors.
    ///
    /// # Parameters
    /// - `bound`: The threshold for the trigger.
    /// - `color`: The color to represent the alert when the threshold is crossed.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with a filled state trigger configured.
    pub fn with_filled_trigger(&self, bound: Trigger<Filled>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_filled.push((bound, color));
        result
    }

    /// Configures triggers based on message latency with associated alert colors.
    ///
    /// # Parameters
    /// - `bound`: The threshold for the trigger.
    /// - `color`: The color to represent the alert when the threshold is crossed.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with a latency trigger configured.
    pub fn with_latency_trigger(&self, bound: Trigger<Duration>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_latency.push((bound, color));
        result
    }


    /// Converts the `ChannelBuilder` configuration into `ChannelMetaData`.
    ///
    /// # Parameters
    /// - `type_name`: The name of the type as a static string.
    /// - `type_byte_count`: The byte count of the type.
    ///
    /// # Returns
    /// A `ChannelMetaData` instance with the configured settings.
    pub(crate) fn to_meta_data(&self, type_name: &'static str, type_byte_count: usize) -> ChannelMetaData {
        assert!(self.capacity > 0);
        let channel_id = self.channel_count.fetch_add(1, Ordering::SeqCst);
        let show_type = if self.show_type {
            Some(type_name.split("::").last().unwrap_or(""))
        } else {
            None
        };

        ChannelMetaData {
            id: channel_id,
            labels: self.labels.into(),
            display_labels: self.display_labels,
            window_bucket_in_bits: self.window_bucket_in_bits,
            refresh_rate_in_bits: self.refresh_rate_in_bits,
            line_expansion: self.line_expansion,
            show_type,
            type_byte_count,
            percentiles_filled: self.percentiles_filled.clone(),
            percentiles_rate: self.percentiles_rate.clone(),
            percentiles_latency: self.percentiles_latency.clone(),
            std_dev_inflight: self.std_dev_filled.clone(),
            std_dev_consumed: self.std_dev_rate.clone(),
            std_dev_latency: self.std_dev_latency.clone(),
            trigger_rate: self.trigger_rate.clone(),
            trigger_filled: self.trigger_filled.clone(),
            trigger_latency: self.trigger_latency.clone(),
            min_filled: self.min_filled,
            max_filled: self.max_filled,
            capacity: self.capacity,
            avg_filled: self.avg_filled,
            avg_rate: self.avg_rate,
            avg_latency: self.avg_latency,
            connects_sidecar: self.connects_sidecar,

        }
    }


    /// Finalizes the channel configuration and creates the channel with the specified settings.
    /// This method ties together all the configured options, applying them to the newly created channel.
    pub const UNSET: u32 = u32::MAX;

    /// Builds and returns a pair of lazy wrappers of the transmitter and receiver with
    /// the current configuration.
    ///
    /// # Returns
    /// A tuple containing the transmitter (`LazySteadyTx<T>`) and receiver (`LazySteadyRx<T>`).
    pub fn build<T>(&self) -> (LazySteadyTx<T>, LazySteadyRx<T>) {
        let lazy_channel = Arc::new(LazyChannel::new(self));
        (LazySteadyTx::<T>::new(lazy_channel.clone()), LazySteadyRx::<T>::new(lazy_channel.clone()))
    }

    /// Builds eager and returns a pair of transmitter and receiver with the current configuration.
    /// For most cases the build() should be used as its lazy and build on the actor thread.
    /// This method is here mostly for testing a normal eager build.
    ///
    /// # Returns
    /// A tuple containing the transmitter (`SteadyTx<T>`) and receiver (`SteadyRx<T>`).
    pub fn eager_build<T>(&self) -> (SteadyTx<T>, SteadyRx<T>) {

        let type_byte_count = std::mem::size_of::<T>();
        let type_string_name = std::any::type_name::<T>();
        let channel_meta_data = Arc::new(self.to_meta_data(type_string_name, type_byte_count));
        let (sender_tx, receiver_tx) = oneshot::channel();
        let (sender_rx, receiver_rx) = oneshot::channel();

        if let Some(mut osv) = self.oneshot_shutdown_vec.try_lock() {
            osv.push(sender_tx);
            osv.push(sender_rx);
        } else {
            let osv_arc = self.oneshot_shutdown_vec.clone();
            let oneshots_future = async move {
                let mut oneshots = osv_arc.lock().await;
                oneshots.push(sender_tx);
                oneshots.push(sender_rx);
            };
            abstract_executor::block_on(oneshots_future);
        }

        let (sender_is_closed, receiver_is_closed) = oneshot::channel();
        let tx_version = Arc::new(AtomicU32::new(Self::UNSET));
        let rx_version = Arc::new(AtomicU32::new(Self::UNSET));
        let noise_threshold = self.noise_threshold;

        let rb = AsyncRb::<ChannelBacking<T>>::new(self.capacity);
        let (tx, rx) = rb.split();
        (
            Arc::new(Mutex::new(Tx {
                tx,
                channel_meta_data: channel_meta_data.clone(),
                local_index: MONITOR_UNKNOWN,
                make_closed: Some(sender_is_closed),
                last_error_send: noise_threshold,
                oneshot_shutdown: receiver_tx,
            })),
            Arc::new(Mutex::new(Rx {
                rx,
                channel_meta_data,
                local_index: MONITOR_UNKNOWN,
                is_closed: receiver_is_closed,
                last_error_send: noise_threshold,
                oneshot_shutdown: receiver_rx,
                rx_version: rx_version.clone(),
                tx_version: tx_version.clone(),
                last_checked_tx_instance: tx_version.load(Ordering::SeqCst),
                take_count: AtomicU32::new(0),
                cached_take_count: AtomicU32::new(0),
                peek_repeats: AtomicUsize::new(0),
                iterator_count_drift: Arc::new(AtomicIsize::new(0)),
            })),
        )
    }
}




/// Postpones the allocation of SteadyTx until the first time clone() is called.
/// This is helpful to ensure the channel is near the active thread
/// 
#[derive(Debug)]
pub struct LazySteadyTx<T> {
    lazy_channel: Arc<LazyChannel<T>>,
}

impl <T> LazySteadyTx<T> {
    fn new(lazy_channel: Arc<LazyChannel<T>>) -> Self {
        LazySteadyTx {
            lazy_channel,
        }
    }
    
    /// Lazy creation of SteadyTx in this clone() call. The same instance is cached for future calls.
    /// This allows for channels to be created more near to their actors.
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyTx<T> {
        nuclei::block_on(self.lazy_channel.get_tx_clone())
    }

    /// For testing simulates sending data to the actor in a controlled manner.
    /// Warning this will send some and wait (block) for them to be consumed before sending more.
    /// 
    pub async fn testing_send_in_two_batches(&self, data: Vec<T>, step_delay: Duration, close: bool) {
        let tx = self.lazy_channel.get_tx_clone().await;
        let mut tx = tx.lock().await;

        let mut trigger:isize = (data.len()/2) as isize;
        //split data into two vec of equal length
        for d in data.into_iter() {
            let _ =  tx.tx.push(d).await;
            trigger -= 1;
            if 0==trigger {
                loop { //ensure all this was processed before moving on for repeatable tests.
                    Delay::new(step_delay).await;
                    if tx.tx.is_empty() {
                        break;
                    }
                }
            }
        }
        if close {
            tx.mark_closed(); // for clean shutdown we tell the actor we have no more data
        }
        loop { //ensure all this was processed before moving on for repeatable tests.
            Delay::new(step_delay).await;
            if tx.tx.is_empty() {
                break;
            }
        }
    }

    /// Simple send of all the data in the vec for testing
    pub async fn testing_send_all(&self, data: Vec<T>, close: bool) {
        let tx = self.lazy_channel.get_tx_clone().await;
        let mut tx = tx.lock().await;
        for d in data.into_iter() {
            let _ =  tx.tx.push(d).await;     
        }
        if close {
            tx.mark_closed(); // for clean shutdown we tell the actor we have no more data
        }        
    }
    
    /// wait duration and then close and wait again
    pub async fn testing_close(&self, step_delay: Duration) {
        Delay::new(step_delay).await;
        let tx = self.lazy_channel.get_tx_clone().await;
        let mut tx = tx.lock().await;
        tx.mark_closed(); // for clean shutdown we tell the actor we have no more data
        Delay::new(step_delay).await;
    }


}


/// Same as SteadyRx however provides clone() to lazy init the SteadyRx
/// If already cloned it returns clone of the SteadyRx.
///
#[derive(Debug)]
pub struct LazySteadyRx<T> {
    lazy_channel: Arc<LazyChannel<T>>,
}
impl <T> LazySteadyRx<T> {
    fn new(lazy_channel: Arc<LazyChannel<T>>) -> Self {
        LazySteadyRx {
            lazy_channel,
        }
    }

    /// Returns a clone of the SteadyRx which is the same one used going forward
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyRx<T> {
        nuclei::block_on(self.lazy_channel.get_rx_clone())
    }

    /// For testing simulates taking data from the actor in a controlled manner.
    pub async fn testing_avail_units(&self) -> usize {
        let rx = self.lazy_channel.get_rx_clone().await;
        let mut rx = rx.lock().await;
        rx.avail_units()
    }


    /// For testing simulates taking data from the actor in a controlled manner.
    pub async fn testing_take(&self) -> Vec<T> {
        let rx = self.lazy_channel.get_rx_clone().await;
        let mut rx = rx.lock().await;
        let limit = rx.capacity();
        rx.shared_take_into_iter().take(limit).collect()
    }

}

#[derive(Debug)]
pub(crate) struct LazyChannel<T> {
    builder: ChannelBuilder,
    channel: Mutex<Option<(SteadyTx<T>, SteadyRx<T>)>>,
}

impl <T> LazyChannel<T> {
    fn new(builder: &ChannelBuilder) -> Self {
        LazyChannel {
            builder: builder.clone(),
            channel: Mutex::new(None),
        }
    }

    pub(crate) async fn get_tx_clone(&self) -> SteadyTx<T> {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            *channel = Some(self.builder.eager_build());
        }
        channel.as_ref().expect("internal error").0.clone()
    }

    pub(crate) async fn get_rx_clone(&self) -> SteadyRx<T> {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            *channel = Some(self.builder.eager_build());
        }
        channel.as_ref().expect("internal error").1.clone()
    }
}



impl Metric for Rate {}

/// Represents a rate of occurrence over time.
///
/// The `Rate` struct is used to express a rate of events per unit of time.
/// Internally, it is represented as a rational number with a numerator (units) and
/// a denominator (time in milliseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    // Internal representation as a rational number of the rate per ms
    // Numerator: units, Denominator: time in ms
    numerator: u64,
    denominator: u64,
}

impl Rate {
    /// Creates a new `Rate` instance representing units per millisecond.
    ///
    /// # Arguments
    ///
    /// * `units` - The number of units occurring per millisecond.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Rate;
    ///
    /// let rate = Rate::per_millis(5);
    /// assert_eq!(rate.rational_ms(), (5, 1));
    /// ```
    pub fn per_millis(units: u64) -> Self {
        Self {
            numerator: units,
            denominator: 1,
        }
    }

    /// Creates a new `Rate` instance representing units per second.
    ///
    /// # Arguments
    ///
    /// * `units` - The number of units occurring per second.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Rate;
    ///
    /// let rate = Rate::per_seconds(5);
    /// assert_eq!(rate.rational_ms(), (5000, 1));
    /// ```
    pub fn per_seconds(units: u64) -> Self {
        Self {
            numerator: units * 1000,
            denominator: 1,
        }
    }

    /// Creates a new `Rate` instance representing units per minute.
    ///
    /// # Arguments
    ///
    /// * `units` - The number of units occurring per minute.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Rate;
    ///
    /// let rate = Rate::per_minutes(5);
    /// assert_eq!(rate.rational_ms(), (300000, 1));
    /// ```
    pub fn per_minutes(units: u64) -> Self {
        Self {
            numerator: units * 1000 * 60,
            denominator:  1,
        }
    }

    /// Creates a new `Rate` instance representing units per hour.
    ///
    /// # Arguments
    ///
    /// * `units` - The number of units occurring per hour.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Rate;
    ///
    /// let rate = Rate::per_hours(5);
    /// assert_eq!(rate.rational_ms(), (18000000, 1));
    /// ```
    pub fn per_hours(units: u64) -> Self {
        Self {
            numerator: units * 1000 * 60 * 60,
            denominator:  1,
        }
    }

    /// Creates a new `Rate` instance representing units per day.
    ///
    /// # Arguments
    ///
    /// * `units` - The number of units occurring per day.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Rate;
    ///
    /// let rate = Rate::per_days(5);
    /// assert_eq!(rate.rational_ms(), (432000000, 1));
    /// ```
    pub fn per_days(units: u64) -> Self {
        Self {
            numerator: units * 1000 * 60 * 60 * 24,
            denominator: 1,
        }
    }

    /// Returns the rate as a rational number (numerator, denominator) to represent the rate per ms.
    /// This method ensures the rate can be used without performing division, preserving precision.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Rate;
    ///
    /// let rate = Rate::per_seconds(5);
    /// assert_eq!(rate.rational_ms(), (5000, 1));
    /// ```
    pub fn rational_ms(&self) -> (u64, u64) {
        (self.numerator, self.denominator)
    }
}


impl Metric for Filled {}

/// Represents different types of fill levels for metrics alerts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filled {
    /// Represents a percentage filled, as numerator and denominator.
    Percentage(u64, u64),
    /// Represents an exact fill level.
    Exact(u64),
}

impl Filled {
    /// Creates a new `Filled` instance representing a percentage filled.
    ///
    /// # Arguments
    ///
    /// * `value` - A floating-point value representing the percentage.
    ///
    /// # Returns
    ///
    /// * `Some(Filled::Percentage)` if the percentage is within the valid range of 0.0 to 100.0.
    /// * `None` if the percentage is outside the valid range.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::percentage(75.0);
    /// assert_eq!(fill, Some(Filled::Percentage(75000, 100000)));
    ///
    /// let invalid_fill = Filled::percentage(150.0);
    /// assert_eq!(invalid_fill, None);
    /// ```
    pub fn percentage(value: f32) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self::Percentage((value * 1_000f32) as u64, 100_000u64))
        } else {
            None
        }
    }

    /// Creates a new `Filled` instance representing a 10% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p10();
    /// assert_eq!(fill, Filled::Percentage(10, 100));
    /// ```
    pub fn p10() -> Self { Self::Percentage(10, 100) }

    /// Creates a new `Filled` instance representing a 20% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p20();
    /// assert_eq!(fill, Filled::Percentage(20, 100));
    /// ```
    pub fn p20() -> Self { Self::Percentage(20, 100) }

    /// Creates a new `Filled` instance representing a 30% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p30();
    /// assert_eq!(fill, Filled::Percentage(30, 100));
    /// ```
    pub fn p30() -> Self { Self::Percentage(30, 100) }

    /// Creates a new `Filled` instance representing a 40% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p40();
    /// assert_eq!(fill, Filled::Percentage(40, 100));
    /// ```
    pub fn p40() -> Self { Self::Percentage(40, 100) }

    /// Creates a new `Filled` instance representing a 50% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p50();
    /// assert_eq!(fill, Filled::Percentage(50, 100));
    /// ```
    pub fn p50() -> Self { Self::Percentage(50, 100) }

    /// Creates a new `Filled` instance representing a 60% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p60();
    /// assert_eq!(fill, Filled::Percentage(60, 100));
    /// ```
    pub fn p60() -> Self { Self::Percentage(60, 100) }

    /// Creates a new `Filled` instance representing a 70% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p70();
    /// assert_eq!(fill, Filled::Percentage(70, 100));
    /// ```
    pub fn p70() -> Self { Self::Percentage(70, 100) }

    /// Creates a new `Filled` instance representing an 80% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p80();
    /// assert_eq!(fill, Filled::Percentage(80, 100));
    /// ```
    pub fn p80() -> Self { Self::Percentage(80, 100) }

    /// Creates a new `Filled` instance representing a 90% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p90();
    /// assert_eq!(fill, Filled::Percentage(90, 100));
    /// ```
    pub fn p90() -> Self { Self::Percentage(90, 100) }

    /// Creates a new `Filled` instance representing a 100% fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::p100();
    /// assert_eq!(fill, Filled::Percentage(100, 100));
    /// ```
    pub fn p100() -> Self { Self::Percentage(100, 100) }

    /// Creates a new `Filled` instance representing an exact fill level.
    ///
    /// # Arguments
    ///
    /// * `value` - An unsigned integer representing the exact fill level.
    ///
    /// # Examples
    ///
    /// ```
    /// use steady_state::Filled;
    ///
    /// let fill = Filled::exact(42);
    /// assert_eq!(fill, Filled::Exact(42));
    /// ```
    pub fn exact(value: u64) -> Self {
        Self::Exact(value)
    }
}

#[cfg(test)]
mod tests_inputs {
    use super::*;

    #[test]
    fn test_rate_per_millis() {
        let rate = Rate::per_millis(5);
        assert_eq!(rate.rational_ms(), (5, 1));
    }

    #[test]
    fn test_rate_per_seconds() {
        let rate = Rate::per_seconds(5);
        assert_eq!(rate.rational_ms(), (5000, 1));
    }

    #[test]
    fn test_rate_per_minutes() {
        let rate = Rate::per_minutes(5);
        assert_eq!(rate.rational_ms(), (300000, 1));
    }

    #[test]
    fn test_rate_per_hours() {
        let rate = Rate::per_hours(5);
        assert_eq!(rate.rational_ms(), (18000000, 1));
    }

    #[test]
    fn test_rate_per_days() {
        let rate = Rate::per_days(5);
        assert_eq!(rate.rational_ms(), (432000000, 1));
    }

    #[test]
    fn test_filled_percentage_valid() {
        assert_eq!(Filled::percentage(75.0), Some(Filled::Percentage(75000, 100000)));
        assert_eq!(Filled::percentage(0.0), Some(Filled::Percentage(0, 100000)));
        assert_eq!(Filled::percentage(100.0), Some(Filled::Percentage(100000, 100000)));
    }

    #[test]
    fn test_filled_percentage_invalid() {
        assert_eq!(Filled::percentage(-1.0), None);
        assert_eq!(Filled::percentage(101.0), None);
    }

    #[test]
    fn test_filled_exact() {
        assert_eq!(Filled::exact(42), Filled::Exact(42));
    }

    #[test]
    fn test_filled_p10() {
        assert_eq!(Filled::p10(), Filled::Percentage(10, 100));
    }

    #[test]
    fn test_filled_p20() {
        assert_eq!(Filled::p20(), Filled::Percentage(20, 100));
    }

    #[test]
    fn test_filled_p30() {
        assert_eq!(Filled::p30(), Filled::Percentage(30, 100));
    }

    #[test]
    fn test_filled_p40() {
        assert_eq!(Filled::p40(), Filled::Percentage(40, 100));
    }

    #[test]
    fn test_filled_p50() {
        assert_eq!(Filled::p50(), Filled::Percentage(50, 100));
    }

    #[test]
    fn test_filled_p60() {
        assert_eq!(Filled::p60(), Filled::Percentage(60, 100));
    }

    #[test]
    fn test_filled_p70() {
        assert_eq!(Filled::p70(), Filled::Percentage(70, 100));
    }

    #[test]
    fn test_filled_p80() {
        assert_eq!(Filled::p80(), Filled::Percentage(80, 100));
    }

    #[test]
    fn test_filled_p90() {
        assert_eq!(Filled::p90(), Filled::Percentage(90, 100));
    }

    #[test]
    fn test_filled_p100() {
        assert_eq!(Filled::p100(), Filled::Percentage(100, 100));
    }
}


#[cfg(test)]
pub(crate) mod test_builder {
    use super::*;
    use std::time::{Instant};
    use crate::actor_builder::Percentile;

    #[test]
    pub(crate) fn test_channel_builder_new() {
        let channel_count = Arc::new(AtomicUsize::new(0));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
        let noise_threshold = Instant::now();
        let frame_rate_ms = 1000;

        let builder = ChannelBuilder::new(channel_count.clone(), oneshot_shutdown_vec.clone(), noise_threshold, frame_rate_ms);

        assert_eq!(builder.capacity, DEFAULT_CAPACITY);
      //  assert_eq!(builder.labels, &[]);
        assert_eq!(builder.frame_rate_ms, frame_rate_ms);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_capacity() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_capacity(128);

        assert_eq!(new_builder.capacity, 128);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_type() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_type();

        assert!(new_builder.show_type);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_avg_filled() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_avg_filled();

        assert!(new_builder.avg_filled);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_filled_max() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_filled_max();

        assert!(new_builder.max_filled);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_filled_min() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_filled_min();

        assert!(new_builder.min_filled);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_avg_rate() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_avg_rate();

        assert!(new_builder.avg_rate);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_avg_latency() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_avg_latency();

        assert!(new_builder.avg_latency);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_labels() {
        let builder = create_test_channel_builder();
        let labels = &["label1", "label2"];
        let new_builder = builder.with_labels(labels, true);

        assert_eq!(new_builder.labels, labels);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_filled_standard_deviation() {
        let builder = create_test_channel_builder();
        let std_dev = StdDev::one();
        let new_builder = builder.with_filled_standard_deviation(std_dev);

        assert_eq!(new_builder.std_dev_filled.len(), 1);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_rate_standard_deviation() {
        let builder = create_test_channel_builder();
        let std_dev = StdDev::one();
        let new_builder = builder.with_rate_standard_deviation(std_dev);

        assert_eq!(new_builder.std_dev_rate.len(), 1);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_latency_standard_deviation() {
        let builder = create_test_channel_builder();
        let std_dev = StdDev::one();
        let new_builder = builder.with_latency_standard_deviation(std_dev);

        assert_eq!(new_builder.std_dev_latency.len(), 1);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_filled_percentile() {
        let builder = create_test_channel_builder();
        let percentile = Percentile::p50();
        let new_builder = builder.with_filled_percentile(percentile);

        assert_eq!(new_builder.percentiles_filled.len(), 1);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_rate_percentile() {
        let builder = create_test_channel_builder();
        let percentile = Percentile::p50();
        let new_builder = builder.with_rate_percentile(percentile);

        assert_eq!(new_builder.percentiles_rate.len(), 1);
    }

    #[test]
    pub(crate) fn test_channel_builder_with_latency_percentile() {
        let builder = create_test_channel_builder();
        let percentile = Percentile::p50();
        let new_builder = builder.with_latency_percentile(percentile);

        assert_eq!(new_builder.percentiles_latency.len(), 1);
    }

    // #[test]
    // fn test_channel_builder_with_rate_trigger() {
    //     let builder = create_test_channel_builder();
    //     let trigger = Trigger::default();
    //     let color = AlertColor::default();
    //     let new_builder = builder.with_rate_trigger(trigger, color);
    //
    //     assert_eq!(new_builder.trigger_rate.len(), 1);
    // }
    //
    // #[test]
    // fn test_channel_builder_with_filled_trigger() {
    //     let builder = create_test_channel_builder();
    //     let trigger = Trigger::default();
    //     let color = AlertColor::default();
    //     let new_builder = builder.with_filled_trigger(trigger, color);
    //
    //     assert_eq!(new_builder.trigger_filled.len(), 1);
    // }
    //
    // #[test]
    // fn test_channel_builder_with_latency_trigger() {
    //     let builder = create_test_channel_builder();
    //     let trigger = Trigger::default();
    //     let color = AlertColor::default();
    //     let new_builder = builder.with_latency_trigger(trigger, color);
    //
    //     assert_eq!(new_builder.trigger_latency.len(), 1);
    // }

    #[test]
    pub(crate) fn test_channel_builder_to_meta_data() {
        let builder = create_test_channel_builder();
        let type_name = "TestType";
        let type_byte_count = 4;
        let meta_data = builder.to_meta_data(type_name, type_byte_count);

        assert_eq!(meta_data.capacity, builder.capacity);
        assert_eq!(meta_data.labels, builder.labels);
    }

    fn create_test_channel_builder() -> ChannelBuilder {
        let channel_count = Arc::new(AtomicUsize::new(0));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
        let noise_threshold = Instant::now();
        let frame_rate_ms = 1000;
        ChannelBuilder::new(channel_count, oneshot_shutdown_vec, noise_threshold, frame_rate_ms)
    }
}
