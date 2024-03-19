use std::sync::Arc;
use futures::lock::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use async_ringbuf::AsyncRb;


pub(crate) type ChannelBacking<T> = Heap<T>;
pub(crate) type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;
pub(crate) type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;



//TODO: we should use static for all telemetry work. (next step)
//      this might lead to a general solution for static in other places
//let mut rb = AsyncRb::<Static<T, 12>>::default().split_ref()
//   AsyncRb::<ChannelBacking<T>>::new(cap).split_ref()

use ringbuf::traits::Split;
use async_ringbuf::wrap::{AsyncCons, AsyncProd};
use bastion::run;
use futures::channel::oneshot;
#[allow(unused_imports)]
use log::*;
use ringbuf::storage::Heap;
use crate::{AlertColor, config, Metric, MONITOR_UNKNOWN, Rx, StdDev, SteadyRx, SteadyRxBundle, SteadyTx, SteadyTxBundle, Trigger, Tx};
use crate::actor_builder::Percentile;
use crate::monitor::ChannelMetaData;


#[derive(Clone)]
pub struct ChannelBuilder {
    channel_count: Arc<AtomicUsize>,
    capacity: usize,
    labels: &'static [& 'static str],
    display_labels: bool,

    refresh_rate_in_bits: u8,
    window_bucket_in_bits: u8, //ma is 1<<window_bucket_in_bits to ensure power of 2

    line_expansion: bool,
    show_type: bool,
    percentiles_filled: Vec<Percentile>, //each is a row
    percentiles_rate: Vec<Percentile>, //each is a row
    percentiles_latency: Vec<Percentile>, //each is a row
    std_dev_filled: Vec<StdDev>, //each is a row
    std_dev_rate: Vec<StdDev>, //each is a row
    std_dev_latency: Vec<StdDev>, //each is a row
    trigger_rate: Vec<(Trigger<Rate>, AlertColor)>, //if used base is green
    trigger_filled: Vec<(Trigger<Filled>, AlertColor)>, //if used base is green
    trigger_latency: Vec<(Trigger<Duration>, AlertColor)>, //if used base is green
    avg_rate: bool,
    avg_filled: bool,
    avg_latency: bool,
    connects_sidecar: bool,
    oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
}
//some ideas to target
// Primary Label - Latency Estimate (80th Percentile): This gives a quick, representative view of the channel's performance under load.
// Secondary Label - Moving Average of In-Flight Messages: This provides a sense of the current load on the channel.
// Tertiary Label (Optional) - Moving Average of Take Rate: This could be included if there's room and if the take rate is a critical performance factor for your application.


const DEFAULT_CAPACITY: usize = 64;

impl ChannelBuilder {


    pub(crate) fn new(channel_count: Arc<AtomicUsize>,
                      oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>
     ) -> ChannelBuilder {

        ChannelBuilder {
            channel_count,
            capacity: DEFAULT_CAPACITY,
            labels: &[],
            display_labels: false,
            oneshot_shutdown_vec,
            refresh_rate_in_bits: 6, // 1<<6 == 64
            window_bucket_in_bits: 5, // 1<<5 == 32

            line_expansion: false,
            show_type: false,
            percentiles_filled: Vec::new(),
            percentiles_rate: Vec::new(),
            percentiles_latency: Vec::new(),
            std_dev_filled: Vec::new(),
            std_dev_rate: Vec::new(),
            std_dev_latency: Vec::new(),
            trigger_rate: Vec::new(),
            trigger_filled: Vec::new(),
            trigger_latency: Vec::new(),
            avg_filled: false,
            avg_rate: false,
            avg_latency: false,
            connects_sidecar: false,
        }
    }

    pub fn with_compute_refresh_window_floor(&self, refresh: Duration, window: Duration) -> Self {
        let result = self.clone();
        //we must compute the refresh rate first before we do the window
        let frames_per_refresh = refresh.as_micros() / (1000u128 * config::TELEMETRY_PRODUCTION_RATE_MS as u128);
        let refresh_in_bits = (frames_per_refresh as f32).log2().ceil() as u8;
        let refresh_in_micros = (1000u128<<refresh_in_bits) * config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        //now compute the window based on our new bucket size
        let buckets_per_window:f32 = window.as_micros() as f32 / refresh_in_micros as f32;
        //find the next largest power of 2
        let window_in_bits = (buckets_per_window).log2().ceil() as u8;
        result.with_compute_refresh_window_bucket_bits(refresh_in_bits, window_in_bits)
    }

    pub fn with_compute_refresh_window_bucket_bits(&self
                                           , refresh_bucket_in_bits:u8
                                           , window_bucket_in_bits: u8) -> Self {
        let mut result = self.clone();
        result.refresh_rate_in_bits = refresh_bucket_in_bits;
        result.window_bucket_in_bits = window_bucket_in_bits;
        result
    }

    pub fn build_as_bundle<T, const GIRTH:usize>(&self) -> (SteadyTxBundle<T, GIRTH>, SteadyRxBundle<T, GIRTH>) {
        let mut tx_vec = Vec::with_capacity(GIRTH); //: Vec<SteadyTx<T>>
        let mut rx_vec = Vec::with_capacity(GIRTH); //: Vec<SteadyRx<T>>

        (0..GIRTH).for_each(|_| {
            let (t,r) = self.build();
            tx_vec.push(t);
            rx_vec.push(r);
        });

        ( Arc::new(tx_vec.try_into().expect("Incorrect length"))
        , Arc::new(rx_vec.try_into().expect("Incorrect length"))    )
    }


    /// Sets the capacity for the channel being built.
    ///
    /// # Parameters
    /// - `capacity`: The maximum number of messages the channel can hold.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with the specified capacity.
    ///
    /// # Notes
    /// Increase capacity to reduce the chance of backpressure in high-throughput scenarios.
    pub fn with_capacity(&self, capacity: usize) -> Self {
        let mut result = self.clone();
        result.capacity = capacity;
        result
    }

    /// Enables type display in telemetry.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with type display enabled.
    ///
    /// # Notes
    /// Useful for debugging and telemetry analysis to identify the data types in channels.
    pub fn with_type(&self) -> Self {
        let mut result = self.clone();
        result.show_type = true;
        result
    }

    /// Enables line expansion in telemetry visualization.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with line expansion enabled.
    ///
    /// # Notes
    /// Line expansion visualizes the volume of data over time, enhancing traceability and insights.
    pub fn with_line_expansion(&self) -> Self {
        let mut result = self.clone();
        result.line_expansion = true;
        result
    }

    /// Enables average calculation for the filled state of channels.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with average filled calculation enabled.
    ///
    /// # Notes
    /// Average calculations are lightweight and provide a basic understanding of channel utilization.
    pub fn with_avg_filled(&self) -> Self {
        let mut result = self.clone();
        result.avg_filled = true;
        result
    }

    /// Enables average rate calculation.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with average rate calculation enabled.
    ///
    /// # Notes
    /// Useful for monitoring the average throughput of channels, identifying bottlenecks or underutilization.
    pub fn with_avg_rate(&self) -> Self {
        let mut result = self.clone();
        result.avg_rate = true;
        result
    }

    /// Enables average latency calculation.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance with average latency calculation enabled.
    ///
    /// # Notes
    /// Latency calculations help in assessing the responsiveness of the system and identifying delays.
    pub fn with_avg_latency(&self) -> Self {
        let mut result = self.clone();
        result.avg_latency = true;
        result
    }


    /// Marks this connection as going to a sidecar, for display purposes.
    ///
    /// # Returns
    /// A `ChannelBuilder` instance configured with this mark
    ///
    /// # Notes
    /// On charts and graphs, these two nodes will be in the same rank more 'near' each other than unrelated nodes.

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
    ///
    /// # Notes
    /// Labels enhance telemetry data with descriptive tags, improving observability.
    pub fn with_labels(&self, labels: &'static [& 'static str], display: bool) -> Self {
        let mut result = self.clone();
        result.labels = if display {labels} else {&[]};
        result
    }

    /// Configures the channel to calculate the standard deviation of the "filled" state.
    ///
    /// # Parameters
    /// - `config`: Configuration for standard deviation calculation.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with filled state standard deviation calculation enabled.
    ///
    /// # Notes
    /// Standard deviation for the filled state provides insights into the variability of the channel's usage over time.
    /// It's a cost-effective way to gauge the consistency of channel capacity utilization without the computational overhead of histograms.
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
    ///
    /// # Notes
    /// This measures the variability in the rate at which messages are sent or received, offering insights into traffic patterns.
    /// Ideal for identifying periods of high volatility in message throughput.
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
    ///
    /// # Notes
    /// Latency standard deviation helps identify fluctuations in message processing times, useful for performance tuning.
    /// It provides a measure of how consistently the system processes messages, pinpointing instability in handling time.
    pub fn with_latency_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_latency.push(config);
        result
    }

    /// Configures the channel to calculate the standard deviation of message latency.
    ///
    /// # Parameters
    /// - `config`: Configuration for standard deviation calculation of latency.
    ///
    /// # Returns
    /// A modified instance of `ChannelBuilder` with latency standard deviation calculation enabled.
    ///
    /// # Notes
    /// Latency standard deviation helps identify fluctuations in message processing times, useful for performance tuning.
    /// It provides a measure of how consistently the system processes messages, pinpointing instability in handling time.
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
    ///
    /// # Notes
    /// Rate percentiles provide a granular view of message throughput, highlighting the variability and extremes in message flow.
    /// Essential for understanding peak and low traffic patterns, assisting in resource allocation and system scaling.
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
    ///
    /// # Notes
    /// Latency percentiles are key to understanding the distribution of message processing times, from best to worst.
    /// They help in identifying and mitigating outliers that could affect user experience or system efficiency.
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
    ///
    /// # Notes
    /// Rate triggers alert when message throughput exceeds or falls below specified thresholds, enabling real-time reaction to traffic anomalies.
    /// Ideal for systems requiring dynamic scaling or immediate notification of unusual activity.
    pub fn with_rate_trigger(&self, bound: Trigger<Rate>, color: AlertColor) -> Self {
        let mut result = self.clone();
        //need sequence number for these triggers?
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
    ///
    /// # Notes
    /// Filled state triggers provide immediate feedback on channel capacity issues, helping prevent backpressure and message loss.
    /// They are crucial for maintaining optimal system performance and ensuring smooth message processing.
    pub fn with_filled_trigger(&self, bound: Trigger<Filled>, color: AlertColor) -> Self {
        let mut result = self.clone();
        //need sequence number for these triggers?
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
    ///
    /// # Notes
    /// Latency triggers are vital for systems where timely message processing is critical, allowing for swift identification and resolution of delays.
    /// They help ensure that service level agreements (SLAs) are met and user satisfaction is maintained.
    pub fn with_latency_trigger(&self, bound: Trigger<Duration>, color: AlertColor) -> Self {
        let mut result = self.clone();
        //need sequence number for these triggers?
        result.trigger_latency.push((bound, color));
        result
    }

    pub(crate) fn to_meta_data(&self, type_name: &'static str, type_byte_count: usize) -> ChannelMetaData {
        assert!(self.capacity > 0);
        let channel_id = self.channel_count.fetch_add(1, Ordering::SeqCst);
        let show_type = if self.show_type {Some(type_name.split("::").last().unwrap_or(""))} else {None};
        //info!("channel_builder::to_meta_data: show_type: {:?} capacity: {}", show_type,self.capacity);
        ChannelMetaData {
            id: channel_id,
            labels: self.labels.into(),
            display_labels: self.display_labels,
            window_bucket_in_bits: self.window_bucket_in_bits,
            refresh_rate_in_bits: self.refresh_rate_in_bits,
            line_expansion: self.line_expansion,
            show_type, type_byte_count,
            percentiles_filled: self.percentiles_filled.clone(),
            percentiles_rate: self.percentiles_rate.clone(),
            percentiles_latency: self.percentiles_latency.clone(),
            std_dev_inflight: self.std_dev_filled.clone(),
            std_dev_consumed: self.std_dev_rate.clone(),
            std_dev_latency: self.std_dev_latency.clone(),

            trigger_rate: self.trigger_rate.clone(),
            trigger_filled: self.trigger_filled.clone(),
            trigger_latency: self.trigger_latency.clone(),



            capacity: self.capacity,
            avg_filled: self.avg_filled,
            avg_rate: self.avg_rate,
            avg_latency: self.avg_latency,
            connects_sidecar: self.connects_sidecar,

            //TODO: deeper review here.
            expects_to_be_monitored: self.window_bucket_in_bits>0
                                      && ( self.display_labels
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
                                    || !self.std_dev_rate.is_empty())

        }
    }

    /// Finalizes the channel configuration and creates the channel with the specified settings.
    /// This method ties together all the configured options, applying them to the newly created channel.

    pub fn build<T>(&self) -> (SteadyTx<T>, SteadyRx<T>) {

        let rb = AsyncRb::<ChannelBacking<T>>::new(self.capacity);
        let (tx, rx) = rb.split();

        let (sender_tx, receiver_tx) = oneshot::channel();
        let (sender_rx, receiver_rx) = oneshot::channel();
        {
            let mut oneshots = run!(self.oneshot_shutdown_vec.lock());
            oneshots.push(sender_tx);
            oneshots.push(sender_rx);
        }

        //TODO: one shot must be created here

        //trace!("channel_builder::build: type_name: {}", std::any::type_name::<T>());

        //the number of bytes consumed by T
        let type_byte_count = std::mem::size_of::<T>(); //TODO: new feature to add
        let type_string_name = std::any::type_name::<T>();
        let channel_meta_data = Arc::new(self.to_meta_data(type_string_name,type_byte_count));

        let (sender_is_closed, receiver_is_closed) = oneshot::channel();

        (  Arc::new(Mutex::new(Tx {
            tx
            , channel_meta_data: channel_meta_data.clone()
            , local_index: MONITOR_UNKNOWN
            , make_closed: Some(sender_is_closed)
            , last_error_send: Instant::now() //TODO: roll back a few seconds..
            , oneshot_shutdown: receiver_tx
        }))
           , Arc::new(Mutex::new(Rx {
            rx
            , channel_meta_data
            , local_index: MONITOR_UNKNOWN
            , is_closed: receiver_is_closed
            , oneshot_shutdown: receiver_rx
        }))
        )

    }

}

impl Metric for Rate {}

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

impl Metric for Filled {}


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

