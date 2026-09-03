#![allow(clippy::type_complexity)]
//! The `channel_builder` module provides builder utilities for creating channels between actors.
//! Channels can be eagerly or lazily initialized, grouped into bundles, and used for testing.
//!
//! This module defines the `ChannelBuilder`, channel families (e.g., steady and stream channels),
//! and macros for constructing and monitoring channels in an actor graph.

// ss[related channel.lazy.defer-allocation]
use std::fmt::Debug;
// ss[related philosophy.structural-hierarchy]
use std::ops::Sub;
// ss[related philosophy.structural-hierarchy]
use ringbuf::storage::Heap;
// ss[related channel.lazy.defer-allocation]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use futures::lock::Mutex;
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};
// ss[related channel.lazy.defer-allocation]
use async_ringbuf::AsyncRb;
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicUsize, Ordering};
// ss[related philosophy.structural-hierarchy]
use crate::core_exec;

/** Type alias for the underlying storage backing of the channel, using a heap-based ring buffer. */
// ss[related channel.lazy.defer-allocation]
pub(crate) type ChannelBacking<T> = Heap<T>;

/** Type alias for the internal sender component of the channel, wrapping an asynchronous producer. */
// ss[related channel.lazy.defer-allocation]
pub(crate) type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;

/** Type alias for the internal receiver component of the channel, wrapping an asynchronous consumer. */
// ss[related channel.lazy.defer-allocation]
pub(crate) type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;

//TODO: 2026, we should use static for all telemetry work.
//      this might lead to a general solution for static in other places
//use ringbuf::storage::Heap;
//use ringbuf::storage::Array;
//use async_ringbuf::traits::Split;

//let rb = AsyncRb::<Heap<u8>>::new(1000).split();
//let rb = AsyncRb::<Array<u8,1000>>::default().split();  //static

// ss[related channel.lazy.defer-allocation]
use async_ringbuf::wrap::{AsyncCons, AsyncProd};
// ss[related philosophy.structural-hierarchy]
use futures::channel::oneshot;
#[allow(unused_imports)]
// ss[related channel.lazy.defer-allocation]
use log::*;
// ss[related philosophy.structural-hierarchy]
use async_ringbuf::traits::Split;
// ss[related philosophy.structural-hierarchy]
use crate::{AlertColor, StdDev, SteadyRx, SteadyTx, Trigger, MONITOR_UNKNOWN};
// ss[related channel.lazy.defer-allocation]
use crate::actor_builder_units::Percentile;
// ss[related philosophy.structural-hierarchy]
use crate::channel_builder_lazy::{LazyChannel, LazySteadyRx, LazySteadyRxBundle, LazySteadyTx, LazySteadyTxBundle};
// ss[related channel.lazy.defer-allocation]
use crate::channel_builder_units::{Filled, Rate};
// ss[related philosophy.structural-hierarchy]
use crate::distributed::aqueduct_stream::{LazySteadyStreamRxBundle, LazySteadyStreamTxBundle, LazyStream, LazyStreamRx, LazyStreamTx, RxChannelMetaDataWrapper, StreamControlItem, TxChannelMetaDataWrapper};
// ss[related philosophy.structural-hierarchy]
use crate::monitor::ChannelMetaData;
// ss[related channel.lazy.defer-allocation]
use crate::steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS;
// ss[related philosophy.structural-hierarchy]
use crate::telemetry_window::compute_refresh_window_frames;
// ss[related philosophy.structural-hierarchy]
use crate::steady_rx::Rx;
// ss[related channel.lazy.defer-allocation]
use crate::steady_tx::Tx;

/**
 * Default capacity for channels unless explicitly set in the builder.
 *
 * This constant defines the default number of messages a channel can hold. Many applications
 * may rely on this value, so it should not be changed. For specific use cases requiring a
 * different capacity, use a custom builder configuration.
 */
// ss[impl channel.default-capacity]
const DEFAULT_CAPACITY: usize = 64; // do not change

/// Heuristic: per-slot dyn estimate larger than `capacity × 1024` likely includes capacity already.
// ss[related channel.dynamic-payload-estimate]
pub(crate) fn dynamic_payload_estimate_looks_capacity_scaled(capacity: usize, bytes_per_slot: usize) -> bool {
    capacity > 0 && bytes_per_slot > capacity.saturating_mul(1024)
}

/**
 * Builder for configuring and creating channels within the Steady State framework.
 *
 * The `ChannelBuilder` provides a flexible interface for configuring channel properties such as
 * capacity, telemetry metrics, triggers, and labels. It supports both eager and lazy initialization
 * of channels, making it suitable for a variety of actor-based communication scenarios.
 */
#[derive(Clone, Debug, Default)]
// ss[related channel.lazy.defer-allocation]
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

    /// Per-slot heap/referenced payload estimate for telemetry dyn display.
    dynamic_per_slot_estimate: Option<usize>,
}

// ss[related channel.lazy.defer-allocation]
impl ChannelBuilder {
    /**
     * Creates a new `ChannelBuilder` instance with default settings.
     *
     * Initializes the builder with a shared channel counter, shutdown sender vector, and frame rate.
     *
     * # IMPORTANT: Telemetry rollups advance once per frame (`frame_rate_ms`) on the metrics
     * collector. Actor and channel rolling windows use the same frame-based bit sizing
     * ([`crate::telemetry_window::compute_refresh_window_frames`]). Sub-sampling in
     * `relay_stats_smartly` only affects how often an actor may send telemetry, not window depth.
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
    // ss[related channel.lazy.defer-allocation]
    pub(crate) fn new(
        channel_count: Arc<AtomicUsize>,
        oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
        frame_rate_ms: u64,
    ) -> ChannelBuilder {
        // Build default refresh/window in *frames*.
        let (refresh_in_bits, window_in_bits) = compute_refresh_window_frames(
            frame_rate_ms as u128,
            Duration::from_secs(1),
            Duration::from_secs(10),
        );

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
            dynamic_per_slot_estimate: None,
        }
    }

    /**
     * Configures the refresh rate and window size for telemetry data collection.
     *
     * Adjusts how frequently telemetry data is refreshed and the size of the aggregation window,
     * optimizing for performance and accuracy based on the provided durations.
     *
     * # IMPORTANT: Channel rollups are per-frame (server already quantizes telemetry into frames).
     * This method sizes the refresh/window in *frames*.
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
    // ss[related channel.lazy.defer-allocation]
    pub fn with_compute_refresh_window_floor(&self, refresh: Duration, window: Duration) -> Self {
        let mut result = self.clone();
        let (refresh_in_bits, window_in_bits) =
            compute_refresh_window_frames(self.frame_rate_ms as u128, refresh, window);
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
    pub fn with_no_totals(&self) -> Self {
        let mut result = self.clone();
        result.show_total = false;
        result
    }

    /**
     * Enables reserved buffer memory display in telemetry and the DOT graph.
     *
     * When enabled, DOT shows **ring** (`capacity × size_of::<T>()`) and optional **dyn**
     * (estimated heap beyond the ring via [`with_dynamic_payload_estimate`](Self::with_dynamic_payload_estimate)).
     * Exposed programmatically via [`Tx::memory_bytes`](crate::steady_tx::Tx::memory_bytes) and
     * [`Tx::dynamic_memory_bytes`](crate::steady_tx::Tx::dynamic_memory_bytes) after locking.
     *
     * **DOT graph display:**
     * - Single channel — edge label gains a `Memory: …B` line.
     * - Partnered channels (`with_partner`) — merged edge header shows the **combined**
     *   footprint of all lanes (e.g. `stream [0] (1KB)`); per-lane footprints appear in
     *   the tooltip.
     * - Bundles — bundle header shows the **summed** footprint of every edge in the bundle;
     *   the same total appears in the bundle tooltip.
     *
     * **Caveat:** footprint is the reserved ring-buffer maximum. It does not include
     * bookkeeping overhead, telemetry structures, or heap payloads behind pointer-sized
     * types (e.g. `Box<[u8]>`).
     *
     * # Returns
     *
     * A new `ChannelBuilder` instance with memory usage display enabled.
     */
    // ss[impl channel.memory-usage-telemetry]
    pub fn with_memory_usage(&self) -> Self {
        let mut result = self.clone();
        result.show_memory = true;
        result
    }

    /// Per-slot estimate of heap/referenced payload beyond the ring (`capacity × per_slot` dyn ceiling).
    ///
    /// # WARNING
    ///
    /// `bytes_per_slot` is **extra** heap per message at full occupancy, not ring slot size.
    /// Do not pass `capacity × per_slot` — capacity is applied again for the dyn total.
    /// Ring memory always remains `capacity × size_of::<T>()`.
    // ss[impl channel.dynamic-payload-estimate]
    pub fn with_dynamic_payload_estimate(&self, bytes_per_slot: usize) -> Self {
        if self.show_memory && dynamic_payload_estimate_looks_capacity_scaled(self.capacity, bytes_per_slot) {
            warn!(
                "with_dynamic_payload_estimate({bytes_per_slot}) looks capacity-scaled for capacity {} \
                 — pass per-slot heap bytes, not ring total",
                self.capacity
            );
        }
        let mut result = self.clone();
        result.dynamic_per_slot_estimate = Some(bytes_per_slot);
        result
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
    pub fn with_filled_max(&self) -> Self {
        let mut result = self.clone();
        result.max_filled = true;
        result
    }

    /// show the max latency
    // ss[related channel.lazy.defer-allocation]
    pub fn with_latency_max(&self) -> Self {
        let mut result = self.clone();
        result.max_latency = true;
        result
    }

    /// show the max rate
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
    pub fn with_filled_min(&self) -> Self {
        let mut result = self.clone();
        result.min_filled = true;
        result
    }

    /// show the min latency
    // ss[related channel.lazy.defer-allocation]
    pub fn with_latency_min(&self) -> Self {
        let mut result = self.clone();
        result.min_latency = true;
        result
    }

    /// show the min rate
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
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
            ring_slot_byte_count: type_byte_count,
            dynamic_per_slot_estimate: self.dynamic_per_slot_estimate,
        }
    }

    /// Finalizes the channel configuration and creates the channel with the specified settings.
    /// This method ties together all the configured options, applying them to the newly created channel.
    // ss[related channel.lazy.defer-allocation]
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
    // ss[related channel.lazy.defer-allocation]
    pub(crate) fn eager_build_internal<T>(&self) -> (Tx<T>, Rx<T>) {
        let now = Instant::now().sub(Duration::from_secs(1 + MAX_TELEMETRY_ERROR_RATE_SECONDS as u64));

        let ring_slot_byte_count = size_of::<T>();
        if self.show_memory
            && self.dynamic_per_slot_estimate.is_some()
            && dynamic_payload_estimate_looks_capacity_scaled(
                self.capacity,
                self.dynamic_per_slot_estimate.unwrap_or(0),
            )
        {
            warn!(
                "channel dynamic_per_slot_estimate looks capacity-scaled for capacity {} \
                 — dyn telemetry may be wrong",
                self.capacity
            );
        }
        let type_string_name = std::any::type_name::<T>();
        let channel_meta_data =
            Arc::new(self.to_meta_data(type_string_name, ring_slot_byte_count));
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
                registry_key: 0,
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
                registry_key: 0,
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
    // ss[impl channel.eager-build-test]
    pub fn eager_build<T>(&self) -> (SteadyTx<T>, SteadyRx<T>) {
        let (tx, rx) = self.eager_build_internal();
        let tx_meta = tx.channel_meta_data.meta_data.clone();
        let rx_meta = rx.channel_meta_data.meta_data.clone();

        let tx_arc = Arc::new(Mutex::new(tx));
        let rx_arc = Arc::new(Mutex::new(rx));

        let tx_key = Arc::as_ptr(&tx_arc) as usize;
        let rx_key = Arc::as_ptr(&rx_arc) as usize;

        if let Some(mut guard) = tx_arc.try_lock() {
            guard.registry_key = tx_key;
        }
        if let Some(mut guard) = rx_arc.try_lock() {
            guard.registry_key = rx_key;
        }

        let mut reg = crate::monitor::METADATA_REGISTRY.write();
        reg.insert(tx_key, tx_meta);
        reg.insert(rx_key, rx_meta);

        (tx_arc, rx_arc)
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
// ss[impl testing.assert-steady-rx]
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
// ss[related channel.lazy.defer-allocation]
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
// ss[related channel.lazy.defer-allocation]
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


// ss[related channel.lazy.defer-allocation]
#[path = "channel_builder_build.rs"]
mod channel_builder_build;

#[cfg(test)]
// ss[related channel.lazy.defer-allocation]
#[path = "channel_builder_tests.rs"]
mod channel_builder_tests;
