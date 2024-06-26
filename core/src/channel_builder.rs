use ringbuf::storage::Heap;
use std::sync::Arc;
use futures::lock::Mutex;
use std::time::{Duration, Instant};
use async_ringbuf::AsyncRb;
use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering};


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
//# use ringbuf::storage::Heap;
use crate::{abstract_executor, AlertColor, Metric, MONITOR_UNKNOWN, StdDev, SteadyRx, SteadyRxBundle, SteadyTx, SteadyTxBundle, Trigger};
use crate::actor_builder::Percentile;
use crate::monitor::ChannelMetaData;
//# use ringbuf::storage::Heap;
use crate::steady_rx::{Rx};
//# use ringbuf::storage::Heap;
use crate::steady_tx::{Tx};

/// Builder for configuring and creating channels within the Steady State framework.
///
/// The `ChannelBuilder` struct allows for the detailed configuration of channels, including
/// performance metrics, thresholds, and other operational parameters. This provides flexibility
/// in tailoring channels to specific requirements and monitoring needs.
#[derive(Clone)]
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

    /// Indicates whether line expansion is enabled.
    ///
    /// If `true`, line expansion features are activated for the channel.
    line_expansion: bool,

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
        ChannelBuilder {
            noise_threshold,
            channel_count,
            capacity: DEFAULT_CAPACITY,
            labels: &[],
            display_labels: false,
            oneshot_shutdown_vec,
            refresh_rate_in_bits: 6, // 1<<6 == 64
            window_bucket_in_bits: 5, // 1<<5 == 32
            line_expansion: false,
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
        let result = self.clone();
        let frames_per_refresh = refresh.as_micros() / (1000u128 * self.frame_rate_ms as u128);
        let refresh_in_bits = (frames_per_refresh as f32).log2().ceil() as u8;
        let refresh_in_micros = (1000u128 << refresh_in_bits) * self.frame_rate_ms as u128;
        let buckets_per_window: f32 = window.as_micros() as f32 / refresh_in_micros as f32;
        let window_in_bits = (buckets_per_window).log2().ceil() as u8;
        result.with_compute_refresh_window_bucket_bits(refresh_in_bits, window_in_bits)
    }

    /// Sets the refresh rate and window bucket size for the channel.
    ///
    /// # Parameters
    /// - `refresh_bucket_in_bits`: Number of bits for the refresh rate.
    /// - `window_bucket_in_bits`: Number of bits for the window bucket size.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with the specified refresh rate and window bucket size.
    pub fn with_compute_refresh_window_bucket_bits(&self, refresh_bucket_in_bits: u8, window_bucket_in_bits: u8) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = refresh_bucket_in_bits;
        result.window_bucket_in_bits = window_bucket_in_bits;
        result
    }

    /// Builds a bundle of channels with the specified girth.
    ///
    /// # Parameters
    /// - `GIRTH`: The number of channels in the bundle.
    ///
    /// # Returns
    /// A tuple containing bundles of transmitters and receivers.
    pub fn build_as_bundle<T, const GIRTH: usize>(&self) -> (SteadyTxBundle<T, GIRTH>, SteadyRxBundle<T, GIRTH>) {
        let mut tx_vec = Vec::with_capacity(GIRTH);
        let mut rx_vec = Vec::with_capacity(GIRTH);

        (0..GIRTH).for_each(|_| {
            let (t, r) = self.build();
            tx_vec.push(t);
            rx_vec.push(r);
        });

        (
            Arc::new(tx_vec.try_into().expect("Incorrect length")),
            Arc::new(rx_vec.try_into().expect("Incorrect length")),
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
    /// # Returns
    /// A `ChannelBuilder` instance with line expansion enabled.
    pub fn with_line_expansion(&self) -> Self {
        let mut result = self.clone();
        result.line_expansion = true;
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
            expects_to_be_monitored: self.window_bucket_in_bits > 0
                && (self.display_labels
                || self.line_expansion
                || show_type.is_some()
                || !self.percentiles_filled.is_empty()
                || !self.percentiles_latency.is_empty()
                || !self.percentiles_rate.is_empty()
                || self.avg_filled
                || self.avg_latency
                || self.avg_rate
                || !self.std_dev_filled.is_empty()
                || !self.std_dev_latency.is_empty()
                || !self.std_dev_rate.is_empty()),
        }
    }

    /// Finalizes the channel configuration and creates the channel with the specified settings.
    /// This method ties together all the configured options, applying them to the newly created channel.
    pub const UNSET: u32 = u32::MAX;

    /// Builds and returns a pair of transmitter and receiver with the current configuration.
    ///
    /// # Returns
    /// A tuple containing the transmitter (`SteadyTx<T>`) and receiver (`SteadyRx<T>`).
    pub fn build<T>(&self) -> (SteadyTx<T>, SteadyRx<T>) {
        let rb = AsyncRb::<ChannelBacking<T>>::new(self.capacity);
        let (tx, rx) = rb.split();
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

        (
            Arc::new(Mutex::new(Tx {
                tx,
                channel_meta_data: channel_meta_data.clone(),
                local_index: MONITOR_UNKNOWN,
                make_closed: Some(sender_is_closed),
                last_error_send: noise_threshold,
                oneshot_shutdown: receiver_tx,
                rx_version: rx_version.clone(),
                tx_version: tx_version.clone(),
                last_checked_rx_instance: rx_version.load(Ordering::SeqCst),
                dedupeset: Default::default(),
            })),
            Arc::new(Mutex::new(Rx {
                rx,
                channel_meta_data,
                local_index: MONITOR_UNKNOWN,
                is_closed: receiver_is_closed,
                oneshot_shutdown: receiver_rx,
                rx_version: rx_version.clone(),
                tx_version: tx_version.clone(),
                last_checked_tx_instance: tx_version.load(Ordering::SeqCst),
                internal_warn_dedupe_set: Default::default(),
                take_count: AtomicU32::new(0),
                cached_take_count: AtomicU32::new(0),
                peek_repeats: AtomicUsize::new(0),
                iterator_count_drift: Arc::new(AtomicIsize::new(0)),
            })),
        )
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
