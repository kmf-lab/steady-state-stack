//! The `channel_builder` module provides builder utilities for creating channels between actors.
//! Channels can be eagerly or lazily initialized, grouped into bundles, and used for testing.
//!
//! This module defines the `ChannelBuilder`, channel families (e.g., steady and stream channels),
//! and macros for constructing and monitoring channels in an actor graph.

use std::fmt::Debug;
use std::ops::Sub;
use ringbuf::storage::Heap;
use std::sync::Arc;
use futures::lock::Mutex;
use std::time::{Duration, Instant};
use async_ringbuf::AsyncRb;
use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicUsize, Ordering};
use crate::core_exec;

/** Type alias for the underlying storage backing of the channel, using a heap-based ring buffer. */
pub(crate) type ChannelBacking<T> = Heap<T>;

/** Type alias for the internal sender component of the channel, wrapping an asynchronous producer. */
pub(crate) type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;

/** Type alias for the internal receiver component of the channel, wrapping an asynchronous consumer. */
pub(crate) type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;

//TODO: 2026, we should use static for all telemetry work.
//      this might lead to a general solution for static in other places
//use ringbuf::storage::Heap;
//use ringbuf::storage::Array;
//use async_ringbuf::traits::Split;

//let rb = AsyncRb::<Heap<u8>>::new(1000).split();
//let rb = AsyncRb::<Array<u8,1000>>::default().split();  //static

use async_ringbuf::wrap::{AsyncCons, AsyncProd};
use futures::channel::oneshot;
#[allow(unused_imports)]
use log::*;
use async_ringbuf::traits::Split;
use crate::{AlertColor, StdDev, SteadyRx, SteadyTx, Trigger, MONITOR_UNKNOWN};
use crate::actor_builder::ActorBuilder;
use crate::actor_builder_units::Percentile;
use crate::channel_builder_lazy::{LazyChannel, LazySteadyRx, LazySteadyRxBundle, LazySteadyTx, LazySteadyTxBundle};
use crate::channel_builder_units::{Filled, Rate};
use crate::distributed::aqueduct_stream::{LazySteadyStreamRxBundle, LazySteadyStreamTxBundle, LazyStream, LazyStreamRx, LazyStreamTx, RxChannelMetaDataWrapper, StreamControlItem, TxChannelMetaDataWrapper};
use crate::monitor::ChannelMetaData;
use crate::steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS;
use crate::steady_rx::Rx;
use crate::steady_tx::Tx;

/**
 * Default capacity for channels unless explicitly set in the builder.
 *
 * This constant defines the default number of messages a channel can hold. Many applications
 * may rely on this value, so it should not be changed. For specific use cases requiring a
 * different capacity, use a custom builder configuration.
 */
const DEFAULT_CAPACITY: usize = 64; // do not change

/**
 * Builder for configuring and creating channels within the Steady State framework.
 *
 * The `ChannelBuilder` provides a flexible interface for configuring channel properties such as
 * capacity, telemetry metrics, triggers, and labels. It supports both eager and lazy initialization
 * of channels, making it suitable for a variety of actor-based communication scenarios.
 */
#[derive(Clone, Debug, Default)]
pub struct ChannelBuilder {
    /// Shared counter for the number of channels created.
    channel_count: Arc<AtomicUsize>,

    /// The maximum number of messages the channel can hold.
    capacity: usize,

    /// Labels associated with the channel for identification in telemetry outputs.
    labels: &'static [&'static str],

    /// Indicates whether the labels should be displayed in telemetry outputs.
    display_labels: bool,

    /// Bit shift value determining the refresh rate for telemetry data updates.
    refresh_rate_in_bits: u8,

    /// Bit shift value determining the window bucket size for metrics aggregation.
    window_bucket_in_bits: u8,

    //TODO: add size to compute bps and make line expansion fixed.
    /// Scale factor for line expansion in telemetry visualization; NaN indicates disabled.
    line_expansion: f32,

    /// Indicates whether the type of data transmitted should be displayed in telemetry outputs.
    show_type: bool,

    /// Percentiles to track for the channel's filled state, each representing a telemetry row.
    percentiles_filled: Vec<Percentile>,

    /// Percentiles to track for the message rate, each representing a telemetry row.
    percentiles_rate: Vec<Percentile>,

    /// Percentiles to track for message latency, each representing a telemetry row.
    percentiles_latency: Vec<Percentile>,

    /// Standard deviations to track for the filled state, each representing a telemetry row.
    std_dev_filled: Vec<StdDev>,

    /// Standard deviations to track for the message rate, each representing a telemetry row.
    std_dev_rate: Vec<StdDev>,

    /// Standard deviations to track for message latency, each representing a telemetry row.
    std_dev_latency: Vec<StdDev>,

    /// Triggers for message rate with associated alert colors; base color is green if used.
    trigger_rate: Vec<(Trigger<Rate>, AlertColor)>,

    /// Triggers for filled state with associated alert colors; base color is green if used.
    trigger_filled: Vec<(Trigger<Filled>, AlertColor)>,

    /// Triggers for message latency with associated alert colors; base color is green if used.
    trigger_latency: Vec<(Trigger<Duration>, AlertColor)>,

    /// Indicates whether to monitor the average filled state of the channel.
    avg_filled: bool,

    /// Indicates whether to monitor the average message rate through the channel.
    avg_rate: bool,

    /// show min rate
    min_rate: bool,

    /// show max rate
    max_rate: bool,

    /// Indicates whether to monitor the average message latency in the channel.
    avg_latency: bool,

    /// show min latency
    min_latency: bool,

    /// show max latency
    max_latency: bool,

    /// Indicates whether the channel connects to a sidecar for additional processing.
    connects_sidecar: bool,

    /// Indicates whether to display total counts in telemetry outputs.
    show_total: bool,

    /// Indicates whether to monitor the maximum filled state of the channel.
    max_filled: bool,

    /// Indicates whether to monitor the minimum filled state of the channel.
    min_filled: bool,

    /// Interval in milliseconds at which telemetry data is updated.
    frame_rate_ms: u64,

    /// Shared vector of one-shot senders for shutdown notifications, wrapped for thread safety.
    oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,

    /// Optional partner name to be stored in metadata.
    partner: Option<&'static str>,

    /// Optional index within a bundle, used for pairing partnered channels.
    bundle_index: Option<usize>,

    /// Number of channels in the bundle, used for rollup display.
    girth: usize,

    /// Indicates whether to display memory usage in telemetry.
    show_memory: bool,
}

impl ChannelBuilder {
    /**
     * Creates a new `ChannelBuilder` instance with default settings.
     *
     * Initializes the builder with a shared channel counter, shutdown sender vector, and frame rate.
     * Default telemetry settings are computed based on the frame rate, providing a 1-second refresh
     * rate and a 10-second window size.
     *
     * # Arguments
     *
     * - `channel_count`: Shared atomic counter tracking the number of channels created.
     * - `oneshot_shutdown_vec`: Shared vector of one-shot senders for shutdown signals.
     * - `frame_rate_ms`: Frame rate in milliseconds for telemetry updates.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with default configurations applied.
     */
    pub(crate) fn new(
        channel_count: Arc<AtomicUsize>,
        oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
        frame_rate_ms: u64,
    ) -> ChannelBuilder {
        //build default window
        let (refresh_in_bits, window_in_bits) = ActorBuilder::internal_compute_refresh_window(frame_rate_ms as u128
                                                                                              , Duration::from_secs(1)
                                                                                              , Duration::from_secs(10));
        ChannelBuilder {
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
            min_rate: false,
            max_rate: false,
            avg_latency: false,
            min_latency: false,
            max_filled: false,
            min_filled: false,
            connects_sidecar: false,
            show_total: true, //default to show total
            frame_rate_ms,
            max_latency: false,
            partner: None,
            bundle_index: None,
            girth: 1,
            show_memory: false,
        }
    }

    /**
     * Configures the refresh rate and window size for telemetry data collection.
     *
     * Adjusts how frequently telemetry data is refreshed and the size of the aggregation window,
     * optimizing for performance and accuracy based on the provided durations.
     *
     * # Arguments
     *
     * - `refresh`: Desired minimum refresh rate as a `Duration`.
     * - `window`: Desired aggregation window size as a `Duration`.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with updated telemetry settings.
     */
    pub fn with_compute_refresh_window_floor(&self, refresh: Duration, window: Duration) -> Self {
        let mut result = self.clone();
        let (refresh_in_bits, window_in_bits) = ActorBuilder::internal_compute_refresh_window(self.frame_rate_ms as u128, refresh, window);
        result.refresh_rate_in_bits = refresh_in_bits;
        result.window_bucket_in_bits = window_in_bits;
        result
    }

    /**
     * Disables telemetry metric collection for the channel.
     *
     * Sets the refresh rate and window size to zero, eliminating telemetry overhead for performance-critical scenarios.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with telemetry disabled.
     */
    pub fn with_no_refresh_window(&self) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = 0;
        result.window_bucket_in_bits = 0;
        result
    }

    /**
     * Disables the display of total counts in telemetry outputs.
     *
     * By default, totals are shown; this method hides them if not needed.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with total counts display disabled.
     */
    pub fn with_no_totals(&self) -> Self {
        let mut result = self.clone();
        result.show_total = false;
        result
    }

    /**
     * Enables the display of memory usage in telemetry outputs.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with memory usage display enabled.
     */
    pub fn with_memory_usage(&self) -> Self {
        let mut result = self.clone();
        result.show_memory = true;
        result
    }

    /**
     * Creates a bundle of channels with the specified number of channels (girth).
     *
     * Consumes the builder to produce a bundle of lazily initialized transmitter and receiver pairs,
     * configured according to the builder’s settings. Resources are allocated only upon first use.
     *
     * # Type Parameters
     *
     * - `T`: Type of data to transmit through the channels.
     * - `GIRTH`: Number of channels in the bundle, specified as a constant.
     *
     * # Returns
     *
     * a tuple of `LazySteadyTxBundle<T, GIRTH>` and `LazySteadyRxBundle<T, GIRTH>` representing the transmitter and receiver bundles.
     *
     * # Panics
     *
     * Panics with an "Internal error, incorrect length" message if the bundle size does not match `GIRTH`.
     */
    pub fn build_channel_bundle<T, const GIRTH: usize>(&self) -> (LazySteadyTxBundle<T, GIRTH>, LazySteadyRxBundle<T, GIRTH>) {
        let mut tx_vec = Vec::with_capacity(GIRTH);
        let mut rx_vec = Vec::with_capacity(GIRTH);

        (0..GIRTH).for_each(|i| {
            let mut indexed_builder = self.clone();
            indexed_builder.bundle_index = Some(i);
            indexed_builder.girth = GIRTH;
            let (t, r) = indexed_builder.build_channel();
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

    /**
     * Creates a bundle of stream channels with the specified number of channels (girth).
     *
     * Similar to `build_channel_bundle`, but tailored for stream channels handling data in a streaming fashion.
     * Channels are lazily initialized, with resources allocated only upon first use.
     *
     * # Type Parameters
     *
     * - `T`: Type of data to transmit, must implement `StreamControlItem`.
     * - `GIRTH`: Number of stream channels in the bundle, specified as a constant.
     *
     * # Arguments
     *
     * - `bytes_per_item`: Number of bytes per item, used to calculate payload channel capacity.
     *
     * # Returns
     *
     * a tuple of `LazySteadyStreamTxBundle<T, GIRTH>` and `LazySteadyStreamRxBundle<T, GIRTH>` representing the transmitter and receiver bundles.
     *
     * # Panics
     *
     * Panics with an "Internal error, incorrect length" message if the bundle size does not match `GIRTH`.
     */
    pub fn build_stream_bundle<T: StreamControlItem, const GIRTH: usize>(&self
                                                                         , bytes_per_item: usize
    ) -> (LazySteadyStreamTxBundle<T, GIRTH>, LazySteadyStreamRxBundle<T, GIRTH>) {
        let mut tx_vec = Vec::with_capacity(GIRTH); //pre-allocate, we know the size now
        let mut rx_vec = Vec::with_capacity(GIRTH); //pre-allocate, we know the size now

        let payload_channel_builder = &self.with_capacity(self.capacity*bytes_per_item);
        (0..GIRTH).for_each(|i| { //TODO: later add custom builders for items vs payload
            let mut indexed_builder = self.clone();
            indexed_builder.bundle_index = Some(i);
            indexed_builder.girth = GIRTH;
            let mut indexed_payload_builder = payload_channel_builder.clone();
            indexed_payload_builder.bundle_index = Some(i);
            indexed_payload_builder.girth = GIRTH;

            let lazy = Arc::new(LazyStream::new(&indexed_builder, &indexed_payload_builder));
            tx_vec.push(LazyStreamTx::<T>::new(lazy.clone()));
            rx_vec.push(LazyStreamRx::<T>::new(lazy.clone()));
        });

        (
            match tx_vec.try_into() {
                Ok(t) => t,
                Err(_) => panic!("Internal error, incorrect length")
            }
            ,
            match rx_vec.try_into() {
                Ok(t) => t,
                Err(_) => panic!("Internal error, incorrect length")
            }
            ,
        )
    }

    /**
     * Creates a single stream channel with the specified configuration.
     *
     * Produces a lazily initialized stream channel for handling data streams, with capacity adjusted based on item size.
     *
     * # Type Parameters
     *
     * - `T`: Type of data to transmit, must implement `StreamControlItem`.
     *
     * # Arguments
     *
     * - `bytes_per_item`: Number of bytes per item, used to calculate payload channel capacity.
     *
     * # Returns
     *
     * a tuple of `LazyStreamTx<T>` and `LazyStreamRx<T>` representing the transmitter and receiver.
     */
    pub fn build_stream<T: StreamControlItem>(&self, bytes_per_item: usize) -> (LazyStreamTx<T>, LazyStreamRx<T>) {
        let bytes_capacity = self.capacity*bytes_per_item;
        let lazy_stream = Arc::new(LazyStream::new(self
                                                   , &self.with_capacity(bytes_capacity)));
        (LazyStreamTx::<T>::new(lazy_stream.clone()), LazyStreamRx::<T>::new(lazy_stream.clone()))
    }

    /**
     * Creates a single channel with lazy initialization.
     *
     * Returns lazy wrappers for the transmitter and receiver, deferring resource allocation until first use.
     * Preferred over `eager_build` for resource efficiency in actor systems.
     *
     * # Type Parameters
     *
     * - `T`: Type of data to transmit through the channel.
     *
     * # Returns
     *
     * a tuple of `LazySteadyTx<T>` and `LazySteadyRx<T>` representing the transmitter and receiver.
     */
    pub fn build_channel<T>(&self) -> (LazySteadyTx<T>, LazySteadyRx<T>) {
        let lazy_channel = Arc::new(LazyChannel::new(self));
        (LazySteadyTx::<T>::new(lazy_channel.clone()), LazySteadyRx::<T>::new(lazy_channel.clone()))
    }

    /**
     * Alias for `build_channel`, providing a simpler method name.
     *
     * # Type Parameters
     *
     * - `T`: Type of data to transmit through the channel.
     *
     * # Returns
     *
     * a tuple of `LazySteadyTx<T>` and `LazySteadyRx<T>` representing the transmitter and receiver.
     */
    pub fn build<T>(&self) -> (LazySteadyTx<T>, LazySteadyRx<T>) {
        let lazy_channel = Arc::new(LazyChannel::new(self));
        (LazySteadyTx::<T>::new(lazy_channel.clone()), LazySteadyRx::<T>::new(lazy_channel.clone()))
    }

    /**
     * Sets the maximum capacity of the channel.
     *
     * Specifies how many messages the channel can hold before blocking or dropping, depending on configuration.
     *
     * # Arguments
     *
     * - `capacity`: Maximum number of messages the channel can hold.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the specified capacity.
     */
    pub fn with_capacity(&self, capacity: usize) -> Self {
        let mut result = self.clone();
        result.capacity = capacity;
        result
    }

    /**
     * Enables display of the channel’s data type in telemetry outputs.
     *
     * Useful for debugging and understanding data flow in monitoring outputs.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with type display enabled.
     */
    pub fn with_type(&self) -> Self {
        let mut result = self.clone();
        result.show_type = true;
        result
    }

    /**
     * Enables line expansion in telemetry visualization with a specified scale factor.
     *
     * Enhances visualization of data trends; scale determines expansion or contraction.
     *
     * # Arguments
     *
     * - `scale`: Scale factor (e.g., 1.0 for default, >1.0 for expansion, <1.0 for contraction).
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with line expansion configured.
     */
    pub fn with_line_expansion(&self, scale: f32) -> Self {
        let mut result = self.clone();
        result.line_expansion = scale;
        result
    }

    /**
     * Sets a partner name for the channel to facilitate pairing based on shared tasks.
     *
     * # Arguments
     * - `partner`: A static string literal identifying the partner or task.
     */
    pub fn with_partner(&self, partner: &'static str) -> Self {
        let mut result = self.clone();
        result.partner = Some(partner);
        result
    }

    /**
     * Enables monitoring of the average filled state.
     *
     * Tracks the average number of messages in the channel over time for telemetry reporting.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with average filled state monitoring enabled.
     */
    pub fn with_avg_filled(&self) -> Self {
        let mut result = self.clone();
        result.avg_filled = true;
        result
    }

    /**
     * Enables monitoring of the maximum filled state.
     *
     * Tracks the highest number of messages held, useful for peak usage analysis.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with maximum filled state monitoring enabled.
     */
    pub fn with_filled_max(&self) -> Self {
        let mut result = self.clone();
        result.max_filled = true;
        result
    }

    /// show the max latency
    pub fn with_latency_max(&self) -> Self {
        let mut result = self.clone();
        result.max_latency = true;
        result
    }

    /// show the max rate
    pub fn with_rate_max(&self) -> Self {
        let mut result = self.clone();
        result.max_rate = true;
        result
    }

    /**
     * Enables monitoring of the minimum filled state.
     *
     * Tracks the lowest number of messages held, indicating low activity or bottlenecks.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with minimum filled state monitoring enabled.
     */
    pub fn with_filled_min(&self) -> Self {
        let mut result = self.clone();
        result.min_filled = true;
        result
    }

    /// show the min latency
    pub fn with_latency_min(&self) -> Self {
        let mut result = self.clone();
        result.min_latency = true;
        result
    }

    /// show the min rate
    pub fn with_rate_min(&self) -> Self {
        let mut result = self.clone();
        result.min_rate = true;
        result
    }


    /**
     * Enables monitoring of the average message rate.
     *
     * Provides insight into typical throughput over time via telemetry.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with average rate monitoring enabled.
     */
    pub fn with_avg_rate(&self) -> Self {
        let mut result = self.clone();
        result.avg_rate = true;
        result
    }

    /**
     * Enables monitoring of the average message latency.
     *
     * Tracks the average time messages spend in the channel for performance tuning.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with average latency monitoring enabled.
     */
    pub fn with_avg_latency(&self) -> Self {
        let mut result = self.clone();
        result.avg_latency = true;
        result
    }

    /**
     * Marks the channel as connecting to a sidecar process.
     *
     * Indicates special handling or routing via a sidecar for display purposes.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the sidecar flag set.
     */
    pub fn connects_sidecar(&self) -> Self {
        let mut result = self.clone();
        result.connects_sidecar = true;
        result
    }

    /**
     * Sets labels for the channel and controls their display in telemetry.
     *
     * Labels aid in identification and categorization; display can be toggled.
     *
     * # Arguments
     *
     * - `labels`: Static slice of string slices representing labels.
     * - `display`: Boolean indicating whether to show labels in telemetry.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with configured labels.
     */
    pub fn with_labels(&self, labels: &'static [&'static str], display: bool) -> Self {
        let mut result = self.clone();
        result.labels = if display { labels } else { &[] };
        result
    }

    /**
     * Adds a standard deviation metric for the filled state.
     *
     * Tracks variability in the channel’s filled state over time.
     *
     * # Arguments
     *
     * - `config`: `StdDev` configuration for the filled state metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    pub fn with_filled_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_filled.push(config);
        result
    }

    /**
     * Adds a standard deviation metric for the message rate.
     *
     * Tracks variability in message throughput over time.
     *
     * # Arguments
     *
     * - `config`: `StdDev` configuration for the rate metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    pub fn with_rate_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_rate.push(config);
        result
    }

    /**
     * Adds a standard deviation metric for message latency.
     *
     * Tracks variability in message latency for performance consistency.
     *
     * # Arguments
     *
     * - `config`: `StdDev` configuration for the latency metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    pub fn with_latency_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_latency.push(config);
        result
    }

    /**
     * Adds a percentile metric for the filled state.
     *
     * Provides distribution insights into the channel’s filled state.
     *
     * # Arguments
     *
     * - `config`: `Percentile` configuration for the filled state metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    pub fn with_filled_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_filled.push(config);
        result
    }

    /**
     * Adds a percentile metric for the message rate.
     *
     * Provides distribution insights into message throughput.
     *
     * # Arguments
     *
     * - `config`: `Percentile` configuration for the rate metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    pub fn with_rate_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_rate.push(config);
        result
    }

    /**
     * Adds a percentile metric for message latency.
     *
     * Ensures most messages meet latency requirements via distribution tracking.
     *
     * # Arguments
     *
     * - `config`: `Percentile` configuration for the latency metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    pub fn with_latency_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_latency.push(config);
        result
    }

    /**
     * Adds a trigger for the message rate with an alert color.
     *
     * Alerts when the rate crosses a threshold, aiding proactive monitoring.
     *
     * # Arguments
     *
     * - `bound`: `Trigger<Rate>` defining the alert condition.
     * - `color`: `AlertColor` to display when triggered.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the trigger added.
     */
    pub fn with_rate_trigger(&self, bound: Trigger<Rate>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_rate.push((bound, color));
        result
    }

    /**
     * Adds a trigger for the filled state with an alert color.
     *
     * Alerts based on channel fullness, indicating backpressure or issues.
     *
     * # Arguments
     *
     * - `bound`: `Trigger<Filled>` defining the alert condition.
     * - `color`: `AlertColor` to display when triggered.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the trigger added.
     */
    pub fn with_filled_trigger(&self, bound: Trigger<Filled>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_filled.push((bound, color));
        result
    }

    /**
     * Adds a trigger for message latency with an alert color.
     *
     * Alerts when latency exceeds a threshold, supporting performance SLAs.
     *
     * # Arguments
     *
     * - `bound`: `Trigger<Duration>` defining the alert condition.
     * - `color`: `AlertColor` to display when triggered.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the trigger added.
     */
    pub fn with_latency_trigger(&self, bound: Trigger<Duration>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_latency.push((bound, color));
        result
    }

    /**
     * Converts the builder configuration into `ChannelMetaData` for telemetry.
     *
     * Finalizes metadata with type information if enabled, used for monitoring.
     *
     * # Arguments
     *
     * - `type_name`: Static string name of the transmitted type.
     * - `type_byte_count`: Size in bytes of the transmitted type.
     *
     * # Returns
     *
     * a `ChannelMetaData` instance encapsulating the builder’s configuration.
     *
     * # Panics
     *
     * Panics if `capacity` is zero, as a valid channel requires a positive capacity.
     */
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
            min_rate: self.min_rate,
            max_rate: self.max_rate,
            min_latency: self.min_latency,
            max_latency: self.max_latency,
            capacity: self.capacity,
            avg_filled: self.avg_filled,
            avg_rate: self.avg_rate,
            avg_latency: self.avg_latency,
            connects_sidecar: self.connects_sidecar,
            partner: self.partner,
            bundle_index: self.bundle_index,
            show_total: self.show_total,
            girth: self.girth,
            show_memory: self.show_memory,
        }
    }

    /// Finalizes the channel configuration and creates the channel with the specified settings.
    /// This method ties together all the configured options, applying them to the newly created channel.
    pub const UNSET: u32 = u32::MAX;

    /**
     * Eagerly builds a channel for internal use, returning raw transmitter and receiver structs.
     *
     * Constructs a channel immediately, setting up telemetry and shutdown mechanisms.
     *
     * # Type Parameters
     *
     * - `T`: Type of data to transmit through the channel.
     *
     * # Returns
     *
     * a tuple of `Tx<T>` and `Rx<T>` representing the raw transmitter and receiver.
     */
    pub(crate) fn eager_build_internal<T>(&self) -> (Tx<T>, Rx<T>) {
        let now = Instant::now().sub(Duration::from_secs(1 + MAX_TELEMETRY_ERROR_RATE_SECONDS as u64));

        let type_byte_count = size_of::<T>();
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
            core_exec::block_on(oneshots_future);
        }

        let (sender_is_closed, receiver_is_closed) = oneshot::channel();
        let tx_version = Arc::new(AtomicU32::new(Self::UNSET));
        let rx_version = Arc::new(AtomicU32::new(Self::UNSET));

        //let rb = AsyncRb::<Heap<u8>>::new(1000).split();;
        //let rb = AsyncRb::<Array<u8,1000>>::default().split();
        let rb = AsyncRb::<ChannelBacking<T>>::new(self.capacity);
        let (tx, rx) = rb.split();
        (
            Tx {
                tx,
                channel_meta_data: TxChannelMetaDataWrapper{ meta_data: Arc::clone(&channel_meta_data)},
                local_monitor_index: MONITOR_UNKNOWN,
                make_closed: Some(sender_is_closed),
                last_error_send: now,
                oneshot_shutdown: receiver_tx,
            },
            Rx {
                rx,
                channel_meta_data: RxChannelMetaDataWrapper{ meta_data: Arc::clone(&channel_meta_data)},
                local_monitor_index: MONITOR_UNKNOWN,
                is_closed: receiver_is_closed,
                last_error_send: now,
                oneshot_shutdown: receiver_rx,
                rx_version: rx_version.clone(),
                tx_version: tx_version.clone(),
                last_checked_tx_instance: tx_version.load(Ordering::SeqCst),
                take_count: AtomicU32::new(0),
                cached_take_count: AtomicU32::new(0),
                peek_repeats: AtomicUsize::new(0),
                iterator_count_drift: Arc::new(AtomicIsize::new(0)),
            },
        )
    }

    /**
     * Eagerly builds a channel, returning wrapped transmitter and receiver.
     *
     * Constructs a channel immediately, primarily for testing; prefer `build` for lazy initialization.
     *
     * # Type Parameters
     *
     * - `T`: Type of data to transmit through the channel.
     *
     * # Returns
     *
     * a tuple of `SteadyTx<T>` and `SteadyRx<T>` representing the transmitter and receiver.
     */
    pub fn eager_build<T>(&self) -> (SteadyTx<T>, SteadyRx<T>) {
        let (tx, rx) = self.eager_build_internal();
        (
            Arc::new(Mutex::new(tx)),
            Arc::new(Mutex::new(rx)),
        )
    }
}

/**
 * Asserts that the number of available units in the receiver equals the expected value.
 *
 * Logs an error and panics if the assertion fails, including file and line number for debugging.
 *
 * # Arguments
 *
 * - `$self`: Expression evaluating to a `LazySteadyRx<T>` reference (e.g., `&instance`).
 * - `$expected`: Expected number of available units (`usize`).
 *
 * # Panics
 *
 * Panics if available units do not match the expected value, with detailed error information.
 */
#[macro_export]
macro_rules! assert_steady_rx_eq_count {
    ($self:expr, $expected:expr) => {{
        let rx = $self.clone();
        let measured = if let Some(mut rx) = rx.try_lock() {
            rx.avail_units()
        } else {
            error!("Unable to lock rx for testing");
            panic!("Unable to lock rx for testing");
        };

        if $expected != measured {
            error!(
                "Assertion failed: {} == {} at {}:{}",
                $expected,
                measured,
                file!(),
                line!()
            );
            panic!(
                "Assertion failed at {}:{}: expected {} == measured {}",
                file!(),
                line!(),
                $expected,
                measured
            );
        }
    }};
}

/**
 * Asserts that the number of available units in the receiver exceeds the expected value.
 *
 * Logs an error and panics if the assertion fails, including file and line number.
 *
 * # Arguments
 *
 * - `$self`: Expression evaluating to a `LazySteadyRx<T>` reference (e.g., `&instance`).
 * - `$expected`: Value that available units should exceed (`usize`).
 *
 * # Panics
 *
 * Panics if available units are not greater than the expected value.
 */
#[macro_export]
macro_rules! assert_steady_rx_gt_count {
    ($self:expr, $expected:expr) => {{
        let rx = $self.clone();
        let measured = if let Some(mut rx) = rx.try_lock() {
            rx.avail_units()
        } else {
            error!("Unable to lock rx for testing");
            panic!("Unable to lock rx for testing");
        };
        if !(measured > $expected) {
            error!(
                "Assertion failed: {} > {} at {}:{}",
                $expected,
                measured,
                file!(),
                line!()
            );
            panic!(
                "Assertion failed at {}:{}: expected {} > measured {}",
                file!(),
                line!(),
                $expected,
                measured
            );
        }
    }};
}

/**
 * Asserts that values taken from the receiver match the expected sequence.
 *
 * Panics if there are insufficient values or mismatches, including file and line number.
 *
 * # Arguments
 *
 * - `$self`: Expression evaluating to a `LazySteadyRx<T>` reference (e.g., `&instance`).
 * - `$expected`: Iterable of expected values (e.g., `Vec<T>`).
 *
 * # Type Constraints
 *
 * - `T: PartialEq + Debug`: Type must support equality comparison and debug formatting.
 *
 * # Panics
 *
 * Panics if values are unavailable or do not match expected values.
 */
#[macro_export]
macro_rules! assert_steady_rx_eq_take {
    ($self:expr, $expected:expr) => {{
        let rx = $self.clone();

       if let Some(mut rx) = rx.try_lock() {

            for ex in $expected.into_iter() {
                match rx.try_take() {
                    None => {
                        error!("Expected value but found none");
                        panic!(
                        "Expected value but none available at {}:{}",
                        file!(),
                        line!()
                    )},
                    Some(taken) => {
                        if !ex.eq(&taken) {
                            error!(
                                "Assertion failed: {:?} == {:?} at {}:{}",
                                ex,
                                taken,
                                file!(),
                                line!()
                            );
                            panic!(
                                "Assertion failed at {}:{}: expected {:?} == taken {:?}",
                                file!(),
                                line!(),
                                ex,
                                taken
                            );
                        }
                    }
                }
            }


        } else {
            error!("Unable to lock rx for testing");
            panic!("Unable to lock rx for testing");
        }


    }};
}

// // Simple helper function for streams
// fn stream_bytes(bytes: &[u8]) -> (StreamEgress, Box<[u8]>) {
//     (StreamEgress::new(bytes.len() as i32), bytes.to_vec().into_boxed_slice())
// }

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
        assert_eq!(rate.rational_ms(), (5, 1000));
    }

    #[test]
    fn test_rate_per_minutes() {
        let rate = Rate::per_minutes(5);
        assert_eq!(rate.rational_ms(), (5, 60000));
    }

    #[test]
    fn test_rate_per_hours() {
        let rate = Rate::per_hours(5);
        assert_eq!(rate.rational_ms(), (5, 3600000));
    }

    #[test]
    fn test_rate_per_days() {
        let rate = Rate::per_days(5);
        assert_eq!(rate.rational_ms(), (5, 86400000));
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

    use crate::actor_builder_units::Percentile;
    use crate::steady_rx::RxMetaDataProvider;
    use crate::distributed::aqueduct_stream::StreamIngress;

    #[test]
    pub(crate) fn test_channel_builder_new() {
        let channel_count = Arc::new(AtomicUsize::new(0));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
        let frame_rate_ms = 1000;

        let builder = ChannelBuilder::new(channel_count.clone(), oneshot_shutdown_vec.clone(), frame_rate_ms);

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

    #[test]
    pub(crate) fn test_channel_builder_to_meta_data() {
        let builder = create_test_channel_builder();
        let type_name = "TestType";
        let type_byte_count = 4;
        let meta_data = builder.to_meta_data(type_name, type_byte_count);

        assert_eq!(meta_data.capacity, builder.capacity);
        assert_eq!(meta_data.labels, builder.labels);
        assert_eq!(meta_data.bundle_index, builder.bundle_index);
    }

    #[test]
    pub(crate) fn test_channel_builder_bundle_index() {
        let builder = create_test_channel_builder();
        let (_, rx_bundle) = builder.build_channel_bundle::<i32, 3>();
        
        for (i, rx) in rx_bundle.iter().enumerate() {
            let meta = rx.clone().meta_data();
            assert_eq!(meta.bundle_index, Some(i));
        }
    }

    #[test]
    pub(crate) fn test_stream_builder_bundle_index() {
        let builder = create_test_channel_builder();
        let (_, rx_bundle) = builder.build_stream_bundle::<StreamIngress, 3>(8);
        
        for (i, rx) in rx_bundle.iter().enumerate() {
            let meta = rx.clone().meta_data();
            assert_eq!(meta.bundle_index, Some(i));
        }
    }

    fn create_test_channel_builder() -> ChannelBuilder {
        let channel_count = Arc::new(AtomicUsize::new(0));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
        let frame_rate_ms = 1000;
        ChannelBuilder::new(channel_count, oneshot_shutdown_vec, frame_rate_ms)
    }
}
