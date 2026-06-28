// ss[related philosophy.single-wake-up]
use std::sync::Arc;

use lazy_static::lazy_static;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::thread::ThreadId;

// ss[related philosophy.single-wake-up]
use crate::actor_builder_units::{MCPU, Percentile, Work};
use crate::channel_builder_units::{Filled, Rate};
// ss[related philosophy.single-wake-up]
use crate::dot::RemoteDetails;
use crate::graph_liveliness::ActorIdentity;
// ss[related philosophy.single-wake-up]
use crate::steady_rx::RxMetaDataProvider;
use crate::steady_tx::TxMetaDataProvider;
use crate::*;

lazy_static! {
    // ss[related philosophy.single-wake-up]
    pub(crate) static ref METADATA_REGISTRY: RwLock<HashMap<usize, Arc<ChannelMetaData>>> =
        RwLock::new(HashMap::new());
}

/// Represents the current status of an actor, including performance metrics and state flags.
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
// ss[related philosophy.single-wake-up]
pub struct ActorStatus {
    /// Unique identifier for the actor.
    pub(crate) ident: ActorIdentity,
    /// Total number of times the actor has been restarted.
    pub(crate) total_count_restarts: u32,
    /// Start time of the current iteration, typically measured in nanoseconds.
    pub(crate) iteration_start: u64,
    /// Accumulated sum of iteration times or counts.
    pub(crate) iteration_sum: u64,
    /// Indicates whether the actor has stopped.
    pub(crate) bool_stop: bool,
    /// Indicates whether the actor is stalled (not yielding).
    pub(crate) is_quiet: bool,
    /// Indicates whether the actor is currently blocking.
    pub(crate) bool_blocking: bool,
    /// Total time spent awaiting, measured in nanoseconds.
    pub(crate) await_total_ns: u64,
    /// Total time spent in unit operations, measured in nanoseconds.
    pub(crate) unit_total_ns: u64,// should not be zero.
    /// Optional information about the thread running the actor.
    pub(crate) thread_info: Option<ThreadInfo>,
    /// Array tracking counts of different operation types.
    pub(crate) calls: [u16; 6],
}

/// Contains information about the thread on which an actor is running.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
// ss[related philosophy.single-wake-up]
pub struct ThreadInfo {
    /// Unique identifier of the thread.
    pub(crate) thread_id: ThreadId,
    #[cfg(feature = "core_display")]
    /// Core on which the thread is running, available if the `core_display` feature is enabled.
    pub(crate) core: i32,
}

/// Index for single read operations in the `calls` array of `ActorStatus`.
// ss[related philosophy.single-wake-up]
pub(crate) const CALL_SINGLE_READ: usize = 0;
/// Index for batch read operations in the `calls` array of `ActorStatus`.
pub(crate) const CALL_BATCH_READ: usize = 1;
/// Index for single write operations in the `calls` array of `ActorStatus`.
// ss[related philosophy.single-wake-up]
pub(crate) const CALL_SINGLE_WRITE: usize = 2;
/// Index for batch write operations in the `calls` array of `ActorStatus`.
pub(crate) const CALL_BATCH_WRITE: usize = 3;
/// Index for miscellaneous operations in the `calls` array of `ActorStatus`.
// ss[related philosophy.single-wake-up]
pub(crate) const CALL_OTHER: usize = 4;
/// Index for wait operations in the `calls` array of `ActorStatus`.
pub(crate) const CALL_WAIT: usize = 5;

/// Metadata configuration for an actor, used for monitoring and performance analysis.
///
/// This struct holds settings and identifiers for tracking an actor's behavior within the Steady State framework.
#[derive(Clone, Default, Debug)]
// ss[related philosophy.single-wake-up]
pub struct ActorMetaData {
    /// Unique identifier for the actor.
    pub(crate) ident: ActorIdentity,
    /// Details for remote communication, present if the actor operates in a distributed system.
    pub(crate) remote_details: Option<RemoteDetails>,
    /// Indicates whether to monitor the average microcontroller processing unit (MCPU) usage.
    pub(crate) avg_mcpu: bool,
    /// Indicates whether to monitor the average work performed by the actor.
    pub(crate) avg_work: bool,
    /// Indicates whether to include thread information in telemetry data.
    pub(crate) show_thread_info: bool,
    /// Percentiles to track for MCPU usage metrics.
    pub percentiles_mcpu: Vec<Percentile>,
    /// Percentiles to track for work metrics.
    pub percentiles_work: Vec<Percentile>,
    /// Standard deviations to track for MCPU usage metrics.
    pub std_dev_mcpu: Vec<StdDev>,
    /// Standard deviations to track for work metrics.
    pub std_dev_work: Vec<StdDev>,
    /// Triggers for MCPU usage that raise alerts with associated colors.
    pub trigger_mcpu: Vec<(Trigger<MCPU>, AlertColor)>,
    /// Triggers for work metrics that raise alerts with associated colors.
    pub trigger_work: Vec<(Trigger<Work>, AlertColor)>,
    /// Bit shift value determining the refresh rate of monitoring data.
    pub refresh_rate_in_bits: u8,
    /// Bit shift value determining the window bucket size for metrics aggregation.
    pub window_bucket_in_bits: u8,
    /// Indicates whether to periodically review the actor's usage.
    pub usage_review: bool,
}

/// Immutable metadata for a communication channel, defining its properties and monitoring settings.
///
/// This struct is finalized during channel creation and used for telemetry and performance tracking.
#[derive(Clone, Default, Debug, PartialEq)]
// ss[related philosophy.single-wake-up]
pub struct ChannelMetaData {
    /// Unique identifier for the channel.
    pub(crate) id: usize,
    /// Descriptive labels for the channel, aiding in identification.
    pub(crate) labels: Vec<&'static str>,
    /// Maximum number of items the channel can hold.
    pub(crate) capacity: usize,
    /// Indicates whether to display labels in telemetry output.
    pub(crate) display_labels: bool,
    /// Factor for expanding line displays in visualizations.
    pub(crate) line_expansion: f32,
    /// Optional type descriptor for display purposes.
    pub(crate) show_type: Option<&'static str>,
    /// Bit shift value for the refresh rate of channel metrics.
    pub(crate) refresh_rate_in_bits: u8,
    /// Bit shift value for the window bucket size in channel metrics aggregation.
    pub(crate) window_bucket_in_bits: u8,
    /// Percentiles to track for the channel's filled state.
    pub(crate) percentiles_filled: Vec<Percentile>,
    /// Percentiles to track for the data rate through the channel.
    pub(crate) percentiles_rate: Vec<Percentile>,
    /// Percentiles to track for latency within the channel.
    pub(crate) percentiles_latency: Vec<Percentile>,
    /// Standard deviations to track for the channel's filled state.
    pub(crate) std_dev_inflight: Vec<StdDev>,
    /// Standard deviations to track for the channel's data rate.
    pub(crate) std_dev_consumed: Vec<StdDev>,
    /// Standard deviations to track for the channel's latency.
    pub(crate) std_dev_latency: Vec<StdDev>,
    /// Triggers for data rate that raise alerts with associated colors.
    pub(crate) trigger_rate: Vec<(Trigger<Rate>, AlertColor)>,
    /// Triggers for filled state that raise alerts with associated colors.
    pub(crate) trigger_filled: Vec<(Trigger<Filled>, AlertColor)>,
    /// Triggers for latency that raise alerts with associated colors.
    pub(crate) trigger_latency: Vec<(Trigger<Duration>, AlertColor)>,
    /// Indicates whether to monitor the average filled state.
    pub(crate) avg_filled: bool,
    /// Indicates whether to monitor the average data rate.
    pub(crate) avg_rate: bool,
    /// Indicates whether to monitor the average latency.
    pub(crate) avg_latency: bool,
    /// Indicates whether to monitor the minimum filled state.
    pub(crate) min_filled: bool,
    /// Indicates whether to monitor the maximum filled state.
    pub(crate) max_filled: bool,
    /// Indicates whether to monitor the minimum rate.
    pub(crate) min_rate: bool,
    /// Indicates whether to monitor the maximum rate.
    pub(crate) max_rate: bool,
    /// Indicates whether to monitor the minimum latency.
    pub(crate) min_latency: bool,
    /// Indicates whether to monitor the maximum latency.
    pub(crate) max_latency: bool,



    /// Indicates whether the channel connects to a sidecar process.
    pub(crate) connects_sidecar: bool,
    /// Optional partner name used to pair channels for shared tasks.
    pub(crate) partner: Option<&'static str>,
    /// Optional index within a bundle, used for pairing partnered channels.
    pub(crate) bundle_index: Option<usize>,
    /// Byte size of the data type transmitted through the channel.
    pub(crate) type_byte_count: usize,
    /// Indicates whether to display total metrics in telemetry.
    pub(crate) show_total: bool,
    /// Number of channels in the bundle, used for rollup display.
    pub(crate) girth: usize,
    /// Indicates whether to display memory usage in telemetry.
    pub(crate) show_memory: bool,
}

/// Type alias for transmitter channel metadata, shared via an atomic reference count.
// ss[related philosophy.single-wake-up]
pub type TxMetaData = Arc<ChannelMetaData>;

/// Provides access to transmitter metadata, facilitating macro usage.
///
/// This trait implementation allows easy retrieval of `TxMetaData` instances.
// ss[related philosophy.single-wake-up]
impl TxMetaDataProvider for TxMetaData {
    /// Returns a clone of the transmitter metadata.
    fn meta_data(&self) -> TxMetaData {
        self.clone()
    }
}

/// Holds a fixed-size array of transmitter metadata instances.
// ss[related philosophy.single-wake-up]
pub struct TxMetaDataHolder<const LEN: usize> {
    /// Array of transmitter metadata.
    pub(crate) array: [TxMetaData; LEN],
}

// ss[related philosophy.single-wake-up]
impl<const LEN: usize> TxMetaDataHolder<LEN> {
    /// Creates a new holder with the specified array of transmitter metadata.
    pub fn new(array: [TxMetaData; LEN]) -> Self {
        TxMetaDataHolder { array }
    }

    /// Returns the array of transmitter metadata.
    // ss[related philosophy.single-wake-up]
    pub fn meta_data(self) -> [TxMetaData; LEN] {
        self.array
    }
}

/// Type alias for receiver channel metadata, shared via an atomic reference count.
// ss[related philosophy.single-wake-up]
pub type RxMetaData = Arc<ChannelMetaData>;

/// Provides access to receiver metadata, facilitating macro usage.
///
/// This trait implementation allows easy retrieval of `RxMetaData` instances.
// ss[related philosophy.single-wake-up]
impl RxMetaDataProvider for Arc<ChannelMetaData> {
    /// Returns a clone of the receiver metadata.
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        self.clone()
    }
}

/// Holds a fixed-size array of receiver metadata instances.
// ss[related philosophy.single-wake-up]
pub struct RxMetaDataHolder<const LEN: usize> {
    /// Array of receiver metadata.
    pub(crate) array: [RxMetaData; LEN],
}

// ss[related philosophy.single-wake-up]
impl<const LEN: usize> RxMetaDataHolder<LEN> {
    /// Creates a new holder with the specified array of receiver metadata.
    pub fn new(array: [RxMetaData; LEN]) -> Self {
        RxMetaDataHolder { array }
    }

    /// Returns the array of receiver metadata.
    // ss[related philosophy.single-wake-up]
    pub fn meta_data(self) -> [RxMetaData; LEN] {
        self.array
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActorMetaData, ActorStatus, ChannelMetaData, RxMetaDataHolder, TxMetaDataHolder,
        CALL_BATCH_READ, CALL_BATCH_WRITE, CALL_OTHER, CALL_SINGLE_READ, CALL_SINGLE_WRITE,
        CALL_WAIT, METADATA_REGISTRY,
    };
    use crate::actor_builder_units::{MCPU, Percentile, Work};
    use crate::channel_builder_units::{Filled, Rate};
    use crate::metrics::{AlertColor, StdDev, Trigger};
    use crate::graph_liveliness::ActorIdentity;
    use crate::steady_rx::RxMetaDataProvider;
    use crate::steady_tx::TxMetaDataProvider;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn tx_and_rx_metadata_provider_clone_metadata() {
        let channel = Arc::new(ChannelMetaData {
            id: 7,
            capacity: 64,
            ..Default::default()
        });
        let tx: super::TxMetaData = channel.clone();
        assert_eq!(TxMetaDataProvider::meta_data(&tx).id, 7);
        assert_eq!(RxMetaDataProvider::meta_data(&channel).id, 7);
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn metadata_holders_return_inner_arrays() {
        let a = Arc::new(ChannelMetaData { id: 1, ..Default::default() });
        let b = Arc::new(ChannelMetaData { id: 2, ..Default::default() });
        let tx_holder = TxMetaDataHolder::new([a.clone()]);
        let rx_holder = RxMetaDataHolder::new([b.clone()]);
        let tx_arr = tx_holder.meta_data();
        let rx_arr = rx_holder.meta_data();
        assert_eq!(tx_arr[0].id, 1);
        assert_eq!(rx_arr[0].id, 2);
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn actor_status_and_metadata_defaults() {
        let status = ActorStatus::default();
        assert_eq!(status.ident, ActorIdentity::default());
        assert!(!status.bool_stop);

        let meta = ActorMetaData::default();
        assert!(meta.trigger_mcpu.is_empty());
        assert!(meta.trigger_work.is_empty());
        assert!(!meta.avg_mcpu);
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn call_index_constants_are_ordered_and_distinct() {
        assert_eq!(CALL_SINGLE_READ, 0);
        assert_eq!(CALL_BATCH_READ, 1);
        assert_eq!(CALL_SINGLE_WRITE, 2);
        assert_eq!(CALL_BATCH_WRITE, 3);
        assert_eq!(CALL_OTHER, 4);
        assert_eq!(CALL_WAIT, 5);
        let indices = [
            CALL_SINGLE_READ,
            CALL_BATCH_READ,
            CALL_SINGLE_WRITE,
            CALL_BATCH_WRITE,
            CALL_OTHER,
            CALL_WAIT,
        ];
        for (i, &a) in indices.iter().enumerate() {
            for (j, &b) in indices.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b);
                }
            }
        }
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn metadata_registry_populated_by_eager_channel_build() {
        use crate::channel_builder::ChannelBuilder;
        use std::sync::Arc;

        let (tx, rx) = ChannelBuilder::default()
            .with_capacity(32)
            .eager_build::<u64>();

        let tx_key = Arc::as_ptr(&tx) as usize;
        let rx_key = Arc::as_ptr(&rx) as usize;

        let reg = METADATA_REGISTRY.read();
        assert!(
            reg.contains_key(&tx_key),
            "eager build must register tx metadata by SteadyTx arc pointer key"
        );
        assert!(
            reg.contains_key(&rx_key),
            "eager build must register rx metadata by SteadyRx arc pointer key"
        );
        assert_eq!(reg.get(&tx_key).unwrap().capacity, 32);
        assert_eq!(reg.get(&rx_key).unwrap().capacity, 32);

        // Telemetry uses lock-free registry lookup; must not fall back to default metadata.
        assert_eq!(tx.meta_data().capacity, 32);
        assert_eq!(rx.meta_data().capacity, 32);
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn actor_metadata_trigger_and_percentile_fields() {
        let meta = ActorMetaData {
            ident: ActorIdentity::new(1, "worker", None),
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: true,
            percentiles_mcpu: vec![Percentile::p90()],
            percentiles_work: vec![Percentile::p50()],
            std_dev_mcpu: vec![StdDev::one()],
            std_dev_work: vec![StdDev::two()],
            trigger_mcpu: vec![(Trigger::AvgAbove(MCPU::m512()), AlertColor::Orange)],
            trigger_work: vec![(Trigger::AvgBelow(Work::p50()), AlertColor::Yellow)],
            refresh_rate_in_bits: 4,
            window_bucket_in_bits: 8,
            usage_review: true,
            ..Default::default()
        };
        assert!(meta.avg_mcpu);
        assert_eq!(meta.trigger_mcpu.len(), 1);
        assert_eq!(meta.percentiles_work.len(), 1);
        assert_eq!(meta.std_dev_mcpu[0].value(), 1.0);
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn channel_metadata_triggers_and_flags_equality() {
        let a = ChannelMetaData {
            id: 3,
            capacity: 128,
            trigger_rate: vec![(Trigger::AvgAbove(Rate::per_seconds(50)), AlertColor::Red)],
            trigger_filled: vec![(Trigger::AvgBelow(Filled::p50()), AlertColor::Yellow)],
            trigger_latency: vec![(
                Trigger::PercentileAbove(Percentile::p99(), Duration::from_millis(10)),
                AlertColor::Orange,
            )],
            avg_filled: true,
            avg_rate: true,
            avg_latency: true,
            max_filled: true,
            show_total: true,
            girth: 2,
            ..Default::default()
        };
        let b = a.clone();
        assert_eq!(a, b);
        assert_eq!(a.trigger_rate.len(), 1);
        assert!(a.avg_latency);
    }

    #[test]
    // ss[verify philosophy.single-wake-up]
    fn actor_status_calls_array_tracks_operations() {
        let mut status = ActorStatus::default();
        status.calls[CALL_SINGLE_READ] = 3;
        status.calls[CALL_BATCH_WRITE] = 7;
        status.unit_total_ns = 100;
        status.await_total_ns = 10;
        assert_eq!(status.calls[CALL_SINGLE_READ], 3);
        assert_eq!(status.calls[CALL_BATCH_WRITE], 7);
    }
}
