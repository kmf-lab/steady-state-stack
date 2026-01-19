use std::ops::*;
use std::time::{Duration, Instant};
use std::sync::Arc;
use num_traits::One;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread::ThreadId;

use crate::*;
use crate::actor_builder_units::{Percentile, Work, MCPU};
use crate::channel_builder_units::{Filled, Rate};
use crate::dot::RemoteDetails;
pub(crate) use crate::graph_liveliness::ActorIdentity;
use crate::monitor_telemetry::{SteadyTelemetryActorSend, SteadyTelemetrySend};
use crate::steady_rx::RxMetaDataProvider;
use crate::steady_tx::TxMetaDataProvider;

use lazy_static::lazy_static;
use parking_lot::RwLock;
use std::collections::HashMap;

lazy_static! {
    pub(crate) static ref METADATA_REGISTRY: RwLock<HashMap<usize, Arc<ChannelMetaData>>> =
        RwLock::new(HashMap::new());
}

/// Represents the current status of an actor, including performance metrics and state flags.
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub struct ActorStatus {
    /// Total number of times the actor has been restarted.
    pub(crate) total_count_restarts: u32,
    /// Start time of the current iteration, typically measured in nanoseconds.
    pub(crate) iteration_start: u64,
    /// Accumulated sum of iteration times or counts.
    pub(crate) iteration_sum: u64,
    /// Indicates whether the actor has stopped.
    pub(crate) bool_stop: bool,
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
pub struct ThreadInfo {
    /// Unique identifier of the thread.
    pub(crate) thread_id: ThreadId,
    #[cfg(feature = "core_display")]
    /// Core on which the thread is running, available if the `core_display` feature is enabled.
    pub(crate) core: i32,
}

/// Index for single read operations in the `calls` array of `ActorStatus`.
pub(crate) const CALL_SINGLE_READ: usize = 0;
/// Index for batch read operations in the `calls` array of `ActorStatus`.
pub(crate) const CALL_BATCH_READ: usize = 1;
/// Index for single write operations in the `calls` array of `ActorStatus`.
pub(crate) const CALL_SINGLE_WRITE: usize = 2;
/// Index for batch write operations in the `calls` array of `ActorStatus`.
pub(crate) const CALL_BATCH_WRITE: usize = 3;
/// Index for miscellaneous operations in the `calls` array of `ActorStatus`.
pub(crate) const CALL_OTHER: usize = 4;
/// Index for wait operations in the `calls` array of `ActorStatus`.
pub(crate) const CALL_WAIT: usize = 5;

/// Metadata configuration for an actor, used for monitoring and performance analysis.
///
/// This struct holds settings and identifiers for tracking an actor's behavior within the Steady State framework.
#[derive(Clone, Default, Debug)]
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
    /// Standard deviations to track for inflight data.
    pub(crate) std_dev_inflight: Vec<StdDev>,
    /// Standard deviations to track for consumed data.
    pub(crate) std_dev_consumed: Vec<StdDev>,
    /// Standard deviations to track for latency.
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
pub type TxMetaData = Arc<ChannelMetaData>;

/// Provides access to transmitter metadata, facilitating macro usage.
///
/// This trait implementation allows easy retrieval of `TxMetaData` instances.
impl TxMetaDataProvider for TxMetaData {
    /// Returns a clone of the transmitter metadata.
    fn meta_data(&self) -> TxMetaData {
        self.clone()
    }
}

/// Holds a fixed-size array of transmitter metadata instances.
pub struct TxMetaDataHolder<const LEN: usize> {
    /// Array of transmitter metadata.
    pub(crate) array: [TxMetaData; LEN],
}

impl<const LEN: usize> TxMetaDataHolder<LEN> {
    /// Creates a new holder with the specified array of transmitter metadata.
    pub fn new(array: [TxMetaData; LEN]) -> Self {
        TxMetaDataHolder { array }
    }

    /// Returns the array of transmitter metadata.
    pub fn meta_data(self) -> [TxMetaData; LEN] {
        self.array
    }
}

/// Type alias for receiver channel metadata, shared via an atomic reference count.
pub type RxMetaData = Arc<ChannelMetaData>;

/// Provides access to receiver metadata, facilitating macro usage.
///
/// This trait implementation allows easy retrieval of `RxMetaData` instances.
impl RxMetaDataProvider for Arc<ChannelMetaData> {
    /// Returns a clone of the receiver metadata.
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        self.clone()
    }
}

/// Holds a fixed-size array of receiver metadata instances.
pub struct RxMetaDataHolder<const LEN: usize> {
    /// Array of receiver metadata.
    pub(crate) array: [RxMetaData; LEN],
}

impl<const LEN: usize> RxMetaDataHolder<LEN> {
    /// Creates a new holder with the specified array of receiver metadata.
    pub fn new(array: [RxMetaData; LEN]) -> Self {
        RxMetaDataHolder { array }
    }

    /// Returns the array of receiver metadata.
    pub fn meta_data(self) -> [RxMetaData; LEN] {
        self.array
    }
}

/// Defines methods for telemetry receivers to manage and access telemetry data.
///
/// This trait ensures that implementations are thread-safe and can be sent across threads.
pub trait RxTel: Send + Sync {
    /// Returns a vector of metadata for all transmitter channels.
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    /// Returns a vector of metadata for all receiver channels.
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    /// Consumes and returns the current actor status, if available.
    fn consume_actor(&self) -> Option<ActorStatus>;

    /// Returns the metadata associated with the actor.
    fn actor_metadata(&self) -> Arc<ActorMetaData>;

    /// Consumes take data into the provided vectors, indicating whether data was consumed.
    fn consume_take_into(
        &self,
        take_send_source: &mut Vec<(i64, i64)>,
        future_take: &mut Vec<i64>,
        future_send: &mut Vec<i64>,
    ) -> bool;

    /// Consumes send data into the provided vectors, indicating whether data was consumed.
    fn consume_send_into(
        &self,
        take_send_source: &mut Vec<(i64, i64)>,
        future_send: &mut Vec<i64>,
    ) -> bool;

    /// Returns an actor receiver definition for the specified version, if available.
    fn actor_rx(&self, version: u32) -> Option<Box<SteadyRx<ActorStatus>>>;

    /// Checks if the telemetry is empty and the channel is closed.
    fn is_empty_and_closed(&self) -> bool;

    /// Checks if the telemetry is currently empty.
    fn is_empty(&self) -> bool;
}

/// Finds the local index corresponding to a global index within the telemetry's inverse local index.
///
/// # Parameters
/// - `telemetry`: Reference to a `SteadyTelemetrySend` instance containing the index mapping.
/// - `goal`: The global index to locate.
///
/// # Returns
/// The local index if found, otherwise `MONITOR_NOT`.
pub(crate) fn find_my_index<const LEN: usize>(telemetry: &SteadyTelemetrySend<LEN>, goal: usize) -> usize {
    let (idx, _) = telemetry.inverse_local_index
        .iter()
        .enumerate()
        .find(|(_, value)| **value == goal)
        .unwrap_or((MONITOR_NOT, &MONITOR_NOT));
    idx
}

/// A guard that updates profiling information upon being dropped.
///
/// This struct ensures that profiling metrics are finalized when it goes out of scope.
pub(crate) struct FinallyRollupProfileGuard<'a> {
    /// Reference to the telemetry sender for updating profiling data.
    pub(crate) st: &'a SteadyTelemetryActorSend,
    /// Start time of the operation being profiled.
    pub(crate) start: Instant,
}

impl Drop for FinallyRollupProfileGuard<'_> {
    /// Updates the await time and decrements the concurrent profile counter when dropped.
    fn drop(&mut self) {
        if self.st.hot_profile_concurrent.fetch_sub(1, Ordering::SeqCst).is_one() {
            let p = self.st.hot_profile.load(Ordering::Relaxed);
            let _ = self.st.hot_profile_await_ns_unit.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |f| Some((f + self.start.elapsed().as_nanos() as u64).saturating_sub(p)),
            );
        }
    }
}

/// Wraps an iterator to track and adjust for drift in item counts.
///
/// This struct monitors the difference between expected and actual yields, updating a shared drift counter.
pub(crate) struct DriftCountIterator<I> {
    /// The underlying iterator being wrapped.
    iter: I,
    /// Number of items expected to be yielded.
    expected_count: usize,
    /// Number of items actually yielded so far.
    actual_count: usize,
    /// Shared counter for tracking cumulative drift across iterations.
    iterator_count_drift: Arc<AtomicIsize>,
}

impl<I> DriftCountIterator<I>
where
    I: Iterator + Send,
{
    /// Creates a new iterator wrapper with the specified expected count and drift counter.
    ///
    /// # Parameters
    /// - `expected_count`: Expected number of items to be yielded.
    /// - `iter`: The iterator to wrap.
    /// - `iterator_count_drift`: Shared counter for tracking drift.
    pub fn new(
        expected_count: usize,
        iter: I,
        iterator_count_drift: Arc<AtomicIsize>,
    ) -> Self {
        DriftCountIterator {
            iter,
            expected_count,
            actual_count: 0,
            iterator_count_drift,
        }
    }
}

impl<I> Drop for DriftCountIterator<I> {
    /// Adjusts the shared drift counter based on the difference between actual and expected counts.
    fn drop(&mut self) {
        let drift = self.actual_count as isize - self.expected_count as isize;
        if drift != 0 {
            self.iterator_count_drift.fetch_add(drift, Ordering::Relaxed);
        }
    }
}

impl<I> Iterator for DriftCountIterator<I>
where
    I: Iterator,
{
    type Item = I::Item;

    /// Yields the next item from the wrapped iterator, incrementing the actual count.
    fn next(&mut self) -> Option<Self::Item> {
        let item = self.iter.next();
        if item.is_some() {
            self.actual_count += 1;
        }
        item
    }
}

#[cfg(test)]
pub(crate) mod monitor_tests {
    use std::any::Any;
    use crate::*;
    use super::*;
    use std::ops::DerefMut;
    use lazy_static::lazy_static;
    use std::sync::{Once, OnceLock};
    use std::time::Duration;
    use futures_timer::Delay;
    use std::sync::Arc;
    use parking_lot::RwLock;
    use futures::channel::oneshot;
    use std::time::Instant;
    use std::sync::atomic::AtomicUsize;
    use crate::channel_builder::ChannelBuilder;
    use crate::core_rx::DoubleSlice;
    use crate::steady_actor::SendOutcome;
    use crate::steady_actor_shadow::SteadyActorShadow;
    use crate::core_tx::TxCore;
    use crate::steady_tx::TxDone;

    lazy_static! {
        static ref INIT: Once = Once::new();
    }

    // Helper method to build tx and rx arguments
    fn build_tx_rx() -> (oneshot::Sender<()>, oneshot::Receiver<()>) {
        oneshot::channel()
    }

    // Test for try_peek
    #[test]
    fn test_try_peek() {
        let (_tx,rx) = create_rx(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.try_peek(&mut rx);
            assert_eq!(result, Some(&1));
        };
    }

    // Test for take_slice
    #[test]
    fn test_take_slice() {
        let (_tx,rx) = create_rx(vec![1, 2, 3, 4, 5]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        let mut monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.take_slice(&mut rx, &mut slice);
            assert_eq!(count.item_count(), 3);
            assert_eq!(slice, [1, 2, 3]);
        };
    }

    // Test for try_peek_slice
    #[test]
    fn test_try_peek_slice() {
        let (_tx,rx) = create_rx(vec![1, 2, 3, 4, 5]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let slice = monitor.peek_slice(&mut rx);
            assert_eq!(slice.total_len(), 5);
            assert_eq!(slice.to_vec(), [1, 2, 3, 4, 5]);
            let mut target = [0; 3];
            slice.copy_into_slice(&mut target);
            assert_eq!(target, [1, 2, 3]);

        };
    }

    // Test is_empty method
    #[test]
    fn test_is_empty() {
        let context = test_steady_context();
        let (_tx,rx) = create_rx::<String>(vec![]); // Creating an empty Rx
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            assert!(monitor.is_empty(&mut rx));
        };
    }

    // Test for is_full
    #[test]
    fn test_is_full() {
        let (tx, _rx) = create_test_channel::<String>(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let monitor = context.into_spotlight([], [&tx]);

        if let Some(mut tx) = tx.try_lock() {
            assert!(!monitor.is_full(&mut tx));
        };
    }

    // Test for vacant_units
    #[test]
    fn test_vacant_units() {
        let (tx, _rx) = create_test_channel::<String>(13);
        let context = test_steady_context();
        let tx = tx.clone();
        let monitor = context.into_spotlight([], [&tx]);

        if let Some(mut tx) = tx.try_lock() {
            let vacant_units = monitor.vacant_units(&mut tx);
            assert_eq!(vacant_units, 13); // Assuming only one unit can be vacant
        };
    }

    // Test for wait_empty
    #[async_std::test]
    async fn test_wait_empty() {
        let (tx, _rx) = create_test_channel::<String>(10);
        let tx  = tx.clone();
        let context = test_steady_context();
        let monitor = context.into_spotlight([], [&tx]);

        if let Some(mut tx) = tx.try_lock() {
            let empty = monitor.wait_empty(&mut tx).await;
            assert!(empty);
        };
    }

    // Test avail_units method
    #[test]
    fn test_avail_units() {
        let (_tx,rx) = create_rx(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = context.into_spotlight_internal([], []);
        // context.into_monitor((context,[],[]);

        if let Some(mut rx) = rx.try_lock() {
            assert_eq!(monitor.avail_units(&mut rx), 3);
        };
    }

    // Test for try_peek_iter
    #[test]
    fn test_try_peek_iter() {
        let (_tx,rx) = create_rx(vec![1, 2, 3, 4, 5]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let mut iter = monitor.try_peek_iter(&mut rx);
            assert_eq!(iter.next(), Some(&1));
            assert_eq!(iter.next(), Some(&2));
            assert_eq!(iter.next(), Some(&3));
        };
    }

    // Test for peek_async_iter
    #[async_std::test]
    async fn test_peek_async_iter() {
        let (_tx,rx) = create_rx(vec![1, 2, 3, 4, 5]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            monitor.wait_avail(&mut rx,3).await;
            let mut iter = monitor.try_peek_iter(&mut rx);
            assert_eq!(iter.next(), Some(&1));
            assert_eq!(iter.next(), Some(&2));
            assert_eq!(iter.next(), Some(&3));
        };
    }

    // Test for peek_async
    #[async_std::test]
    async fn test_peek_async() {
        let (_tx,rx) = create_rx(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.peek_async(&mut rx).await;
            assert_eq!(result, Some(&1));
        };
    }

    // Test for send_slice_until_full
    #[test]
    fn test_send_slice_until_full() {
        let (tx, rx) = create_test_channel(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let rx = rx.clone();

        let mut monitor = context.into_spotlight([], [&tx]);

        let slice = [1, 2, 3];
        if let Some(mut tx) = tx.try_lock() {
            let done = monitor.send_slice(&mut tx, &slice);
            assert_eq!(done.item_count(), slice.len());
            if let Some(mut rx) = rx.try_lock() {
                assert_eq!(monitor.try_take(&mut rx), Some(1));
                assert_eq!(monitor.try_peek(&mut rx), Some(&2));
            }

        };
    }

    // Test for try_send
    #[test]
    fn test_try_send() {
        let (tx, _rx) = create_test_channel(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let mut monitor = context.into_spotlight([], [&tx]);

        if let Some(mut tx) = tx.try_lock() {
            let result = match monitor.try_send(&mut tx, 42) {
                SendOutcome::Success => {true}
                SendOutcome::Blocked(_) => {false}
            };
            assert!(result);
        };
    }

    // Common function to create a test SteadyContext
    fn test_steady_context() -> SteadyActorShadow {
        let (_tx, rx) = build_tx_rx();
        SteadyActorShadow {
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
                Default::default(),
                Default::default()
            ))),
            channel_count: Arc::new(AtomicUsize::new(0)),
            ident: ActorIdentity::new(0, "test_actor", None),
            args: Arc::new(Box::new(())),
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())),
            actor_metadata: Arc::new(ActorMetaData::default()),
            oneshot_shutdown_vec: Arc::new(Mutex::new(Vec::new())),
            oneshot_shutdown: Arc::new(Mutex::new(rx)),
            node_tx_rx: None,
            regeneration: 0,
            last_periodic_wait: Default::default(),
            is_in_graph: true,
            actor_start_time: Instant::now(),
            frame_rate_ms: 1000,
            team_id: 0,
            show_thread_info: false,
            aeron_meda_driver: OnceLock::new(),
            use_internal_behavior: true,
            shutdown_barrier: None,

        }
    }

    // Helper function to create a new Rx instance
    fn create_rx<T: std::fmt::Debug>(data: Vec<T>) -> (Arc<Mutex<Tx<T>>>,Arc<Mutex<Rx<T>>>) {
        let (tx, rx) = create_test_channel(10);

        let send = tx.clone();
        if let Some(ref mut send_guard) = send.try_lock() {
            for item in data {
                let _ = send_guard.shared_try_send(item);
            }
        }
        (tx.clone(),rx.clone())
    }

    fn create_test_channel<T: Debug>(capacity: usize) -> (LazySteadyTx<T>, LazySteadyRx<T>) {
        let builder = ChannelBuilder::new(
            Arc::new(Default::default()),
            Arc::new(Default::default()),
            40).with_capacity(capacity);

        builder.build_channel::<T>()
    }

    #[test]
    fn test_simple_monitor_build() {
        let context = test_steady_context();
        let monitor = context.into_spotlight([], []);
        assert_eq!("test_actor",monitor.ident.label.name);
    }

    #[test]
    fn test_macro_monitor_build() {
        let context = test_steady_context();
        let monitor = context.into_spotlight([], []);
        assert_eq!("test_actor",monitor.ident.label.name);

    }

    /// Unit test for relay_stats_tx_custom.
    #[async_std::test]
    async fn test_relay_stats_tx_rx_custom() {
        let _ = logging_util::steady_logger::initialize();

        let mut graph = GraphBuilder::for_testing().build("");
        let (tx_string, rx_string) = graph.channel_builder().with_capacity(8).build_channel();
        let tx_string = tx_string.clone();
        let rx_string = rx_string.clone();

        let context = graph.new_testing_test_monitor("test");
        let mut monitor = context.into_spotlight([&rx_string], [&tx_string]);

        let mut rxd = rx_string.lock().await;
        let mut txd = tx_string.lock().await;

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(&mut txd, "test".to_string(), SendSaturation::WarnThenAwait).await;
            count += 1;
        }

        if let Some(ref mut tx) = monitor.telemetry.send_tx {
            assert_eq!(tx.count[txd.local_monitor_index], threshold);
        }

        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
        monitor.relay_stats_smartly();

        if let Some(ref mut tx) = monitor.telemetry.send_tx {
            assert_eq!(tx.count[txd.local_monitor_index], 0);
        }

        while count > 0 {
            let x = monitor.take_async(&mut rxd).await;
            assert_eq!(x, Some("test".to_string()));
            count -= 1;
        }

        if let Some(ref mut rx) = monitor.telemetry.send_rx {
            assert_eq!(rx.count[rxd.local_monitor_index], threshold);
        }

        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;

        monitor.relay_stats_smartly();

        if let Some(ref mut rx) = monitor.telemetry.send_rx {
            assert_eq!(rx.count[rxd.local_monitor_index], 0);
        }
    }

    /// Unit test for relay_stats_tx_rx_batch.
    #[async_std::test]
    async fn test_relay_stats_tx_rx_batch() {
        let _ = logging_util::steady_logger::initialize();

        let mut graph = GraphBuilder::for_testing().build("");
        let monitor = graph.new_testing_test_monitor("test");

        let (tx_string, rx_string) = graph.channel_builder().with_capacity(5).build_channel();
        let tx_string = tx_string.clone();
        let rx_string = rx_string.clone();

        let mut monitor = monitor.into_spotlight([&rx_string], [&tx_string]);

        let mut rx_string_guard = rx_string.lock().await;
        let mut tx_string_guard = tx_string.lock().await;

        let rxd: &mut Rx<String> = rx_string_guard.deref_mut();
        let txd: &mut Tx<String> = tx_string_guard.deref_mut();

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string(), SendSaturation::WarnThenAwait).await;
            count += 1;
            if let Some(ref mut tx) = monitor.telemetry.send_tx {
                assert_eq!(tx.count[txd.local_monitor_index], count);
            }
        }
        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
        monitor.relay_stats_smartly();

        if let Some(ref mut_tx) = monitor.telemetry.send_tx {
            assert_eq!(mut_tx.count[txd.local_monitor_index], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Some("test".to_string()));
            count -= 1;
        }
        if let Some(ref mut rx) = monitor.telemetry.send_rx {
            assert_eq!(rx.count[rxd.local_monitor_index], threshold);
        }
        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;

        monitor.relay_stats_smartly();

        if let Some(ref mut rx) = monitor.telemetry.send_rx {
            assert_eq!(rx.count[rxd.local_monitor_index], 0);
        }
    }

    // Test for send_iter_until_full
    #[test]
    fn test_send_iter_until_full() {
        let (tx, rx) = create_test_channel(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let rx = rx.clone();
        let mut monitor = context.into_spotlight([], [&tx]);

        let iter = vec![1, 2, 3].into_iter();
        if let Some(mut tx) = tx.try_lock() {
            let sent_count = monitor.send_iter_until_full(&mut tx, iter);
            assert_eq!(sent_count, 3);
            if let Some(mut rx) = rx.try_lock() {
                let i = monitor.take_into_iter(&mut rx);
                assert_eq!(i.collect::<Vec<i32>>(), vec![1, 2, 3]);
            }
        };
    }

    // Test for take_into_iter
    #[test]
    fn test_take_into_iter() {
        let data = vec![1, 2, 3, 4, 5];
        let (tx1, rx1) = create_test_channel(5);

        let tx = tx1.clone();
        if let Some(ref mut send_guard) = tx.try_lock() {
            for item in data {
                let _ = send_guard.shared_try_send(item);
            }
        }
        let rx = rx1.clone();
        let context = test_steady_context();
        let mut monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            {
                assert_eq!(5, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(1));
                assert_eq!(iter.next(), Some(2));
                // we stop early to test if we can continue later
            }
            {//ensure we can take from where we left off
                assert_eq!(3, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(3));
                assert_eq!(iter.next(), Some(4));

            }
        };

        //we still have 1 from before
        if let Some(ref mut send_guard) = tx.try_lock() {
            for item in [6, 7, 8] {
                let _ = send_guard.shared_try_send(item);
            }
        };


        if let Some(mut rx) = rx.try_lock() {
            {
                assert_eq!(4, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(5));
                assert_eq!(iter.next(), Some(6));
                // we stop early to test if we can continue later
            }
            {//ensure we can take from where we left off
                assert_eq!(2, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(7));
                assert_eq!(iter.next(), Some(8));
            }
        };
        if let Some(ref mut send_guard) = tx.try_lock() {
            for item in [9, 10, 11, 12] {
                let _ = send_guard.shared_try_send(item);
            }
        };
        if let Some(mut rx) = rx.try_lock() {
            {
                assert_eq!(4, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(9));
                assert_eq!(iter.next(), Some(10));
                //   drop(iter);
                //inject new data while we have an iterator open
                if let Some(ref mut send_guard) = tx.try_lock() {
                    assert_eq!(1, send_guard.vacant_units());
                    for item in [13, 14, 15] {

                        if let Ok(d) = send_guard.shared_try_send(item) {
                            assert_eq!(TxDone::Normal(1), d );
                        }
                    }
                    assert_eq!(0, send_guard.vacant_units());

                };
            }

        };

    }

    // Test for wait_shutdown_or_avail_units_bundle
    #[async_std::test]
    async fn test_wait_shutdown_or_avail_units_bundle() {
        let context = test_steady_context();
        let (_tx1,rx1) = create_rx(vec![1, 2]);
        let (_tx2,rx2) = create_rx(vec![3, 4]);

        let monitor = context.into_spotlight([&rx1, &rx2], []);
        let mut rx_bundle = RxBundle::new();
        if let Some(rx1) = rx1.try_lock() {
            rx_bundle.push(rx1);
        }
        if let Some(rx2) = rx2.try_lock() {
            rx_bundle.push(rx2);
        }

        let result = monitor
            .wait_avail_bundle(&mut rx_bundle, 2, 2)
            .await;
        assert!(result);
    }

    // Test for wait_closed_or_avail_units_bundle
    #[async_std::test]
    async fn test_wait_closed_or_avail_units_bundle() {
        let context = test_steady_context();
        let (_tx1,rx1) = create_rx(vec![1, 2]);
        let (_tx2,rx2) = create_rx(vec![3, 4]);
        let monitor = context.into_spotlight([&rx1, &rx2], []);

        let mut rx_bundle = RxBundle::new();
        if let Some(rx1) = rx1.try_lock() {
            rx_bundle.push(rx1);
        }
        if let Some(rx2) = rx2.try_lock() {
            rx_bundle.push(rx2);
        }

        let result = monitor
            .wait_avail_bundle(&mut rx_bundle, 2, 2)
            .await;
        assert!(result);
    }

    // Test for wait_avail_units_bundle
    #[async_std::test]
    async fn test_wait_avail_units_bundle() {
        let context = test_steady_context();
        let (_tx1,rx1) = create_rx(vec![1, 2]);
        let (_tx2,rx2) = create_rx(vec![3, 4]);
        let monitor = context.into_spotlight([&rx1, &rx2], []);

        let mut rx_bundle = RxBundle::new();
        if let Some(rx1) = rx1.try_lock() {
            rx_bundle.push(rx1);
        }
        if let Some(rx2) = rx2.try_lock() {
            rx_bundle.push(rx2);
        }

        let result = monitor.wait_avail_bundle(&mut rx_bundle, 2, 2).await;
        assert!(result);
    }

    // Test for wait_shutdown_or_vacant_units_bundle
    #[async_std::test]
    async fn test_wait_shutdown_or_vacant_units_bundle() {
        let (tx1, _rx1) = create_test_channel::<i32>(10);
        let (tx2, _rx2) = create_test_channel::<i32>(10);
        let context = test_steady_context();

        let tx1 =tx1.clone();
        let tx2 =tx2.clone();

        let monitor = context.into_spotlight([], [&tx1, &tx2]);

        let mut tx_bundle = TxBundle::new();
        if let Some(tx1) = tx1.try_lock() {
            tx_bundle.push(tx1);
        }
        if let Some(tx2) = tx2.try_lock() {
            tx_bundle.push(tx2);
        }

        let result = monitor
            .wait_vacant_bundle(&mut tx_bundle, 5, 2)
            .await;
        assert!(result);
    }

    // Test for wait_vacant_units_bundle
    #[async_std::test]
    async fn test_wait_vacant_units_bundle() {
        let (tx1, _rx1) = create_test_channel::<i32>(10);
        let (tx2, _rx2) = create_test_channel::<i32>(10);
        let context = test_steady_context();
        let tx1 =tx1.clone();
        let tx2 =tx2.clone();
        let monitor = context.into_spotlight([], [&tx1, &tx2]);

        let mut tx_bundle = TxBundle::new();
        if let Some(tx1) = tx1.try_lock() {
            tx_bundle.push(tx1);
        }
        if let Some(tx2) = tx2.try_lock() {
            tx_bundle.push(tx2);
        }

        let result = monitor.wait_vacant_bundle(&mut tx_bundle, 5, 2).await;
        assert!(result);
    }

    // Test for wait_shutdown
    #[async_std::test]
    async fn test_wait_shutdown() {
        let context = test_steady_context();
        let monitor = context.into_spotlight([], []);
        // Simulate shutdown
        {
            {
                let mut liveliness = monitor.runtime_state.write();
                liveliness.state = GraphLivelinessState::Running;
            }
            GraphLiveliness::internal_request_shutdown(monitor.runtime_state.clone()).await;
        }
        assert!(monitor.wait_shutdown().await);
    }

    // Test for wait_periodic
    #[async_std::test]
    async fn test_wait_periodic() {
        let context = test_steady_context();
        let monitor = context.into_spotlight([], []);

        let duration = Duration::from_millis(100);
        let result = monitor.wait_periodic(duration).await;
        assert!(result);
    }

    // Test for wait
    #[async_std::test]
    async fn test_wait() {
        let context = test_steady_context();
        let monitor = context.into_spotlight([], []);

        let duration = Duration::from_millis(100);
        let start = Instant::now();
        monitor.wait(duration).await;
        let elapsed = start.elapsed();
        assert!(elapsed >= duration);
    }

    // Test for wait_shutdown_or_avail_units
    #[async_std::test]
    async fn test_wait_shutdown_or_avail_units() {
        let (_tx,rx) = create_rx::<i32>(vec![1,2]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.wait_avail(&mut rx, 1).await;
            assert!(result);
        };
    }

    // Test for wait_closed_or_avail_units
    #[async_std::test]
    async fn test_wait_closed_or_avail_units() {
        let (_tx,rx) = create_rx::<i32>(vec![1]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.wait_avail(&mut rx, 1).await;
            assert!(result);
        };
    }

    // Test for wait_avail_units
    #[async_std::test]
    async fn test_wait_avail_units() {
        let (_tx,rx) = create_rx::<i32>(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.wait_avail(&mut rx, 3).await;
            assert!(result);
        };
    }

    // Test for send_async
    #[async_std::test]
    async fn test_send_async() {
        let (tx, _rx) = create_test_channel::<i32>(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let mut monitor = context.into_spotlight([], [&tx]);

        if let Some(mut tx) = tx.try_lock() {
            let result = monitor.send_async(&mut tx, 42, SendSaturation::WarnThenAwait).await;
            assert!(result.is_sent());
        };
    }

    // Test for args method
    #[test]
    fn test_args() {
        let args = 42u32;
        let context = test_steady_context_with_args(args);
        let monitor = context.into_spotlight([], []);

        let retrieved_args: Option<&u32> = monitor.args();
        assert_eq!(retrieved_args, Some(&42u32));
    }

    // Test for identity method
    #[test]
    fn test_identity() {
        let context = test_steady_context();
        let monitor = context.into_spotlight([], []);

        let identity = monitor.identity();
        assert_eq!(identity.label.name, "test_actor");
    }

    // Helper function to create a test context with arguments
    fn test_steady_context_with_args<A: Any + Send + Sync>(args: A) -> SteadyActorShadow {
        let (_tx, rx) = build_tx_rx();
        SteadyActorShadow {
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
                Default::default(),
                Default::default(),
            ))),
            channel_count: Arc::new(AtomicUsize::new(0)),
            ident: ActorIdentity::new(0, "test_actor", None),
            args: Arc::new(Box::new(args)),
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())),
            actor_metadata: Arc::new(ActorMetaData::default()),
            oneshot_shutdown_vec: Arc::new(Mutex::new(Vec::new())),
            oneshot_shutdown: Arc::new(Mutex::new(rx)),
            node_tx_rx: None,
            regeneration: 0,
            last_periodic_wait: Default::default(),
            is_in_graph: true,
            actor_start_time: Instant::now(),
            frame_rate_ms: 1000,
            team_id: 0,
            show_thread_info: false,
            aeron_meda_driver: OnceLock::new(),
            use_internal_behavior: true,
            shutdown_barrier: None,

        }
    }

    // Test for is_liveliness_in
    #[test]
    fn test_is_liveliness_in() {
        let context = test_steady_context();
        let monitor = context.into_spotlight([], []);

        // Initially, the liveliness state should be Building
        assert!(monitor.is_liveliness_in(&[GraphLivelinessState::Building]));

    }

    // Test for yield_now
    #[async_std::test]
    async fn test_yield_now() {
        let context = test_steady_context();
        let monitor = context.into_spotlight([], []);

        monitor.yield_now().await;
        // If it didn't hang, the test passes
        assert!(true);
    }


    // Test for wait_shutdown_or_avail_units with closed channel
    #[async_std::test]
    async fn test_wait_shutdown_or_avail_units_closed_channel() {
        let (tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let rx = rx.clone();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut tx) = tx.try_lock() {
            tx.mark_closed();
        }

        if let Some(mut rx) = rx.try_lock() {

            let result = monitor.wait_avail(&mut rx, 1).await;
            assert!(!result);
        };
    }

    // Test for wait_shutdown_or_vacant_units with shutdown requested
    // #[async_std::test]
    // async fn test_wait_shutdown_or_vacant_units_shutdown() {
    //     let (tx, _rx) = create_test_channel::<i32>(1);
    //     let context = test_steady_context();
    //     let tx = tx.clone();
    //     let monitor = context.into_monitor([], [&tx]);
    //
    //     // Request shutdown
    //     {
    //         let mut liveliness = monitor.runtime_state.write();
    //         liveliness.state = GraphLivelinessState::Running;
    //         liveliness.request_shutdown();
    //     }
    //
    //     if let Some(mut tx) = tx.try_lock() {
    //         let result = monitor.wait_vacant(&mut tx, 1).await;
    //         assert!(result);
    //     };
    // }


    // Test for args method with String
    #[test]
    fn test_args_string() {
        let args = "test_args".to_string();
        let context = test_steady_context_with_args(args.clone());
        let monitor = context.into_spotlight([], []);

        let retrieved_args: Option<&String> = monitor.args();
        assert_eq!(retrieved_args, Some(&args));
    }

    // Test for take_slice with empty channel
    #[test]
    fn test_take_slice_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        let mut monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.take_slice(&mut rx, &mut slice);
            assert_eq!(count.item_count(), 0);
        };
    }

    // Test for send_slice_until_full with full channel
    #[test]
    fn test_send_slice_until_full_full_channel() {
        let (tx, _rx) = create_test_channel::<i32>(1);
        let context = test_steady_context();
        let tx = tx.clone();
        let mut monitor = context.into_spotlight([], [&tx]);

        if let Some(mut tx) = tx.try_lock() {
            // Fill the channel
            let _ = monitor.try_send(&mut tx, 1);

            // Now the channel is full
            let slice = [2, 3, 4];
            let done = monitor.send_slice(&mut tx, &slice);
            assert_eq!(done.item_count(), 0);
        };
    }

    // Test for take_into_iter with empty channel
    #[test]
    fn test_take_into_iter_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let mut monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let mut iter = monitor.take_into_iter(&mut rx);
            assert_eq!(iter.next(), None);
        };
    }

    // Test for try_peek_slice with empty channel
    #[test]
    fn test_try_peek_slice_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let slice = monitor.peek_slice(&mut rx);
            assert_eq!(slice.total_len(), 0);
        };
    }

    // Test for try_take with empty channel
    #[test]
    fn test_try_take_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let mut monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.try_take(&mut rx);
            assert_eq!(result, None);
        };
    }

    // Test for is_empty when channel has elements
    #[test]
    fn test_is_empty_with_elements() {
        let (_tx,rx) = create_rx::<i32>(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            assert!(!monitor.is_empty(&mut rx));
        };
    }

    // Test for is_full when channel is full
    #[test]
    fn test_is_full_when_full() {
        let (tx, _rx) = create_test_channel::<i32>(1);
        let context = test_steady_context();
        let tx = tx.clone();
        let mut monitor = context.into_spotlight([], [&tx]);

        if let Some(mut tx) = tx.try_lock() {
            // Fill the channel
            let _ = monitor.try_send(&mut tx, 1);
            assert!(monitor.is_full(&mut tx));
        };
    }

    // Test for wait_closed_or_avail_units with closed channel
    #[async_std::test]
    async fn test_wait_closed_or_avail_units_closed_channel() {
        let (tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let monitor = context.into_spotlight([&rx], []);

        if let Some(mut tx) = tx.try_lock() {
            tx.mark_closed();
        }
        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.wait_avail(&mut rx, 1).await;
            assert!(!result);
        };
    }
}
