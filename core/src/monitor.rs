use std::ops::*;
use std::time::{Duration, Instant};
use std::sync::Arc;
use num_traits::{One};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread::ThreadId;

use crate::*;
use crate::actor_builder::{Percentile, Work, MCPU};
use crate::channel_builder::{Filled, Rate};
use crate::dot::RemoteDetails;
use crate::graph_liveliness::{ActorIdentity};
use crate::monitor_telemetry::{SteadyTelemetryActorSend, SteadyTelemetrySend};
use crate::steady_rx::RxMetaDataProvider;
use crate::steady_tx::TxMetaDataProvider;

/// Represents the status of an actor.
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub struct ActorStatus {
    pub(crate) total_count_restarts: u32,
    pub(crate) iteration_start: u64,
    pub(crate) iteration_sum: u64,
    pub(crate) bool_stop: bool,
    pub(crate) await_total_ns: u64,
    pub(crate) unit_total_ns: u64,
    pub(crate) thread_info: Option<ThreadInfo>,
    pub(crate) calls: [u16; 6],
}

/// All the thread data to show for this actor
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadInfo {
    pub(crate) thread_id: ThreadId,
    pub(crate) team_id:   usize,
    #[cfg(feature = "core_display")]
    pub(crate) core: i32,
}

pub(crate) const CALL_SINGLE_READ: usize = 0;
pub(crate) const CALL_BATCH_READ: usize = 1;
pub(crate) const CALL_SINGLE_WRITE: usize = 2;
pub(crate) const CALL_BATCH_WRITE: usize = 3;
pub(crate) const CALL_OTHER: usize = 4;
pub(crate) const CALL_WAIT: usize = 5;

/// Metadata for an actor.
///
/// The `ActorMetaData` struct contains information and configurations related to an actor within the
/// Steady State framework. This metadata is used to monitor and manage the performance and behavior of the actor.
#[derive(Clone, Default, Debug)]
pub struct ActorMetaData {
    /// The unique identifier for the actor.
    ///
    pub(crate) ident: ActorIdentity,

    /// If this actor is part of an aqueduct spanning to other machines this will be populated
    /// To indicate where this actor is sending or receiving data.
    pub(crate) remote_details: Option<RemoteDetails>,

    /// Indicates whether the average microcontroller processing unit (MCPU) usage is monitored.
    ///
    /// If `true`, the average MCPU usage is tracked for this actor.
    pub(crate) avg_mcpu: bool,

    /// Indicates whether the average work performed by the actor is monitored.
    ///
    /// If `true`, the average work is tracked for this actor.
    pub(crate) avg_work: bool,

    /// A list of percentiles for the MCPU usage.
    ///
    /// This list defines various percentiles to be tracked for the MCPU usage of the actor.
    pub percentiles_mcpu: Vec<Percentile>,

    /// A list of percentiles for the work performed by the actor.
    ///
    /// This list defines various percentiles to be tracked for the work metrics of the actor.
    pub percentiles_work: Vec<Percentile>,

    /// A list of standard deviations for the MCPU usage.
    ///
    /// This list defines various standard deviation metrics to be tracked for the MCPU usage of the actor.
    pub std_dev_mcpu: Vec<StdDev>,

    /// A list of standard deviations for the work performed by the actor.
    ///
    /// This list defines various standard deviation metrics to be tracked for the work metrics of the actor.
    pub std_dev_work: Vec<StdDev>,

    /// A list of triggers for the MCPU usage with associated alert colors.
    ///
    /// This list defines conditions (triggers) for the MCPU usage that, when met, will raise alerts of specific colors.
    pub trigger_mcpu: Vec<(Trigger<MCPU>, AlertColor)>,

    /// A list of triggers for the work performed by the actor with associated alert colors.
    ///
    /// This list defines conditions (triggers) for the work metrics that, when met, will raise alerts of specific colors.
    pub trigger_work: Vec<(Trigger<Work>, AlertColor)>,

    /// The refresh rate for monitoring data, expressed in bits.
    ///
    /// This field defines how frequently the monitoring data should be refreshed.
    pub refresh_rate_in_bits: u8,

    /// The size of the window bucket for metrics, expressed in bits.
    ///
    /// This field defines the size of the window bucket used for metrics aggregation.
    pub window_bucket_in_bits: u8,

    /// Indicates whether usage review is enabled for the actor.
    ///
    /// If `true`, the actor's usage is periodically reviewed.
    pub usage_review: bool,
    
}


/// Metadata for a channel, which is immutable once built.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct ChannelMetaData {
    pub(crate) id: usize,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) capacity: usize,
    pub(crate) display_labels: bool,
    pub(crate) line_expansion: f32,
    pub(crate) show_type: Option<&'static str>,
    pub(crate) refresh_rate_in_bits: u8,
    pub(crate) window_bucket_in_bits: u8,
    pub(crate) percentiles_filled: Vec<Percentile>,
    pub(crate) percentiles_rate: Vec<Percentile>,
    pub(crate) percentiles_latency: Vec<Percentile>,
    pub(crate) std_dev_inflight: Vec<StdDev>,
    pub(crate) std_dev_consumed: Vec<StdDev>,
    pub(crate) std_dev_latency: Vec<StdDev>,
    pub(crate) trigger_rate: Vec<(Trigger<Rate>, AlertColor)>,
    pub(crate) trigger_filled: Vec<(Trigger<Filled>, AlertColor)>,
    pub(crate) trigger_latency: Vec<(Trigger<Duration>, AlertColor)>,
    pub(crate) avg_filled: bool,
    pub(crate) avg_rate: bool,
    pub(crate) avg_latency: bool,
    pub(crate) min_filled: bool,
    pub(crate) max_filled: bool,
    pub(crate) connects_sidecar: bool,
    pub(crate) type_byte_count: usize,
    pub(crate) show_total: bool,
}

/// Metadata for a transmitter channel.

pub type TxMetaData = Arc<ChannelMetaData>;
/// supports the macro as an easy way to get the metadata
impl TxMetaDataProvider for TxMetaData {
    fn meta_data(&self) -> TxMetaData {
        self.clone()
    }
}

pub struct TxMetaDataHolder<const LEN: usize> {
    pub(crate) array:[TxMetaData;LEN]
}
impl <const LEN: usize>TxMetaDataHolder<LEN> {
    pub fn new(array: [TxMetaData;LEN]) -> Self {
        TxMetaDataHolder { array }
    }
    pub fn meta_data(self) -> [TxMetaData;LEN] {
        self.array
    }
}

/// Metadata for a receiver channel.
pub type RxMetaData = Arc<ChannelMetaData>;
/// supports the macro as an easy way to get the metadata
impl RxMetaDataProvider for Arc<ChannelMetaData> {
    fn meta_data(&self) -> Arc<ChannelMetaData> { self.clone() }
}
pub struct RxMetaDataHolder<const LEN: usize> {
    pub(crate) array:[RxMetaData;LEN]
}
impl <const LEN: usize>RxMetaDataHolder<LEN> {
    pub fn new(array: [RxMetaData;LEN]) -> Self {
        RxMetaDataHolder { array }
    }

    pub fn meta_data(self) -> [RxMetaData;LEN] {
        self.array
    }
}



/// Trait for telemetry receiver.
pub trait RxTel: Send + Sync {
    /// Returns a vector of channel metadata for transmitter channels.
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    /// Returns a vector of channel metadata for receiver channels.
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    /// Consumes actor status and returns it.
    fn consume_actor(&self) -> Option<ActorStatus>;

    /// Returns the metadata of the actor.
    fn actor_metadata(&self) -> Arc<ActorMetaData>;

    /// Consumes take data into the provided vectors.
    fn consume_take_into(&self, take_send_source: &mut Vec<(i64, i64)>, future_take: &mut Vec<i64>, future_send: &mut Vec<i64>) -> bool;

    /// Consumes send data into the provided vectors.
    fn consume_send_into(&self, take_send_source: &mut Vec<(i64, i64)>, future_send: &mut Vec<i64>) -> bool;

    /// Returns an actor receiver definition for the specified version.
    fn actor_rx(&self, version: u32) -> Option<Box<SteadyRx<ActorStatus>>>;

    /// Checks if the telemetry is empty and closed.
    fn is_empty_and_closed(&self) -> bool;
}



/// Finds the index of a given goal in the telemetry inverse local index.
///
/// # Parameters
/// - `telemetry`: A reference to a `SteadyTelemetrySend` instance.
/// - `goal`: The goal index to find.
///
/// # Returns
/// The index of the goal if found, otherwise returns `MONITOR_NOT`.
pub(crate) fn find_my_index<const LEN: usize>(telemetry: &SteadyTelemetrySend<LEN>, goal: usize) -> usize {
    let (idx, _) = telemetry.inverse_local_index
        .iter()
        .enumerate()
        .find(|(_, value)| **value == goal)
        .unwrap_or((MONITOR_NOT, &MONITOR_NOT));
    idx
}


pub(crate) struct FinallyRollupProfileGuard<'a> {
    pub(crate) st: &'a SteadyTelemetryActorSend,
    pub(crate) start: Instant,
}

impl Drop for FinallyRollupProfileGuard<'_> {
    fn drop(&mut self) {
        // this is ALWAYS run so we need to wait until our concurrent count is back down to zero
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

pub(crate) struct DriftCountIterator<I> {
    iter: I,
    expected_count: usize,
    actual_count: usize,
    iterator_count_drift: Arc<AtomicIsize>,
}

impl<I> DriftCountIterator<I>
where
    I: Iterator + Send,
{
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
    use std::sync::Once;
    use std::time::Duration;
    use futures_timer::Delay;
    use std::sync::Arc;
    use parking_lot::RwLock;
    use futures::channel::oneshot;
    use std::time::Instant;
    use std::sync::atomic::AtomicUsize;
    use crate::channel_builder::ChannelBuilder;
    use crate::commander_context::SteadyContext;
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
        let monitor = context.into_monitor([&rx],[]);

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
        let mut monitor = context.into_monitor([&rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.take_slice(&mut rx, &mut slice);
            assert_eq!(count, 3);
            assert_eq!(slice, [1, 2, 3]);
        };
    }

    // Test for try_peek_slice
    #[test]
    fn test_try_peek_slice() {
        let (_tx,rx) = create_rx(vec![1, 2, 3, 4, 5]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        let monitor = context.into_monitor([&rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.try_peek_slice(&mut rx, &mut slice);
            assert_eq!(count, 3);
            assert_eq!(slice, [1, 2, 3]);
        };
    }


    // Test is_empty method
    #[test]
    fn test_is_empty() {
        let context = test_steady_context();
        let (_tx,rx) = create_rx::<String>(vec![]); // Creating an empty Rx
        let monitor = context.into_monitor([&rx],[]);

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
        let monitor = context.into_monitor([],[&tx]);

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
        let monitor = context.into_monitor([],[&tx]);

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
        let monitor = context.into_monitor([],[&tx]);

        if let Some(mut tx) = tx.try_lock() {
            let empty = monitor.wait_empty(&mut tx).await;
            println!("Empty: {}", empty);
            println!("Vacant units: {}", monitor.vacant_units(&mut tx));
            println!("Capacity: {}", tx.capacity());
                
            assert!(empty);
        };
    }


    // Test avail_units method
    #[test]
    fn test_avail_units() {
        let (_tx,rx) = create_rx(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = context.into_monitor_internal([],[]);
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
        let monitor = context.into_monitor([&rx],[]);

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
        let monitor = context.into_monitor([&rx],[]);

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
        let monitor = context.into_monitor([&rx],[]);

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

        let mut monitor = context.into_monitor([],[&tx]);

        let slice = [1, 2, 3];
        if let Some(mut tx) = tx.try_lock() {
            let sent_count = monitor.send_slice_until_full(&mut tx, &slice);
            assert_eq!(sent_count, slice.len());
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
        let mut monitor = context.into_monitor([],[&tx]);

        if let Some(mut tx) = tx.try_lock() {
            let result = monitor.try_send(&mut tx, 42);
            assert!(result.is_ok());
        };
    }




    // Test for call_async
    #[async_std::test]
    async fn test_call_async() {
        let context = test_steady_context();
        let monitor = context.into_monitor([],[]);

        let fut = async { 42 };
        let result = monitor.call_async(fut).await;
        assert_eq!(result, Some(42));
    }

    // Common function to create a test SteadyContext
    fn test_steady_context() -> SteadyContext {
        let (_tx, rx) = build_tx_rx();
        SteadyContext {
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
            instance_id: 0,
            last_periodic_wait: Default::default(),
            is_in_graph: true,
            actor_start_time: Instant::now(),
            frame_rate_ms: 1000,
            team_id: 0,
            show_thread_info: false,
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
            Instant::now(),
            40).with_capacity(capacity);

        builder.build::<T>()
    }

    #[test]
    fn test_simple_monitor_build() {
        let context = test_steady_context();
        let monitor = context.into_monitor([],[]);
        assert_eq!("test_actor",monitor.ident.label.name);
    }

    #[test]
    fn test_macro_monitor_build() {
        let context = test_steady_context();
        let monitor = context.into_monitor([],[]);
        assert_eq!("test_actor",monitor.ident.label.name);

    }


    /// Unit test for relay_stats_tx_custom.
    #[async_std::test]
    async fn test_relay_stats_tx_rx_custom() {
        util::logger::initialize();

        let mut graph = GraphBuilder::for_testing().build("");
        let (tx_string, rx_string) = graph.channel_builder().with_capacity(8).build();
        let tx_string = tx_string.clone();
        let rx_string = rx_string.clone();

        let context = graph.new_testing_test_monitor("test");
        let mut monitor = context.into_monitor([&rx_string], [&tx_string]);

        let mut rxd = rx_string.lock().await;
        let mut txd = tx_string.lock().await;

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(&mut txd, "test".to_string(), SendSaturation::Warn).await;
            count += 1;
        }

        if let Some(ref mut tx) = monitor.telemetry.send_tx {
            assert_eq!(tx.count[txd.local_index], threshold);
        }

        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
        monitor.relay_stats_smartly();

        if let Some(ref mut tx) = monitor.telemetry.send_tx {
            assert_eq!(tx.count[txd.local_index], 0);
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
        util::logger::initialize();

        let mut graph = GraphBuilder::for_testing().build("");
        let monitor = graph.new_testing_test_monitor("test");

        let (tx_string, rx_string) = graph.channel_builder().with_capacity(5).build();
        let tx_string = tx_string.clone();
        let rx_string = rx_string.clone();

        let mut monitor = monitor.into_monitor([&rx_string], [&tx_string]);

        let mut rx_string_guard = rx_string.lock().await;
        let mut tx_string_guard = tx_string.lock().await;

        let rxd: &mut Rx<String> = rx_string_guard.deref_mut();
        let txd: &mut Tx<String> = tx_string_guard.deref_mut();

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string(), SendSaturation::Warn).await;
            count += 1;
            if let Some(ref mut tx) = monitor.telemetry.send_tx {
                assert_eq!(tx.count[txd.local_index], count);
            }
        }
        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
        monitor.relay_stats_smartly();

        if let Some(ref mut tx) = monitor.telemetry.send_tx {
            assert_eq!(tx.count[txd.local_index], 0);
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
        let mut monitor = context.into_monitor([],[&tx]);

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
        let mut monitor = context.into_monitor([&rx],[]);

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
                        
                        match send_guard.shared_try_send(item) {
                            Ok(d) => {
                                assert_eq!(TxDone::Normal(1), d );
                            },
                            Err(_) => {}
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
        
        let monitor = context.into_monitor([&rx1, &rx2], []);
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
        let monitor = context.into_monitor([&rx1, &rx2], []);

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
        let monitor = context.into_monitor([&rx1, &rx2], []);

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
        
        let monitor = context.into_monitor([], [&tx1, &tx2]);

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
        let monitor = context.into_monitor([], [&tx1, &tx2]);

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
        let monitor = context.into_monitor([], []);

        // Simulate shutdown
        {
            let mut liveliness = monitor.runtime_state.write();
            liveliness.request_shutdown();
        }

        let result = monitor.wait_shutdown().await;
        assert!(result);
    }

    // Test for wait_periodic
    #[async_std::test]
    async fn test_wait_periodic() {
        let context = test_steady_context();
        let monitor = context.into_monitor([], []);

        let duration = Duration::from_millis(100);
        let result = monitor.wait_periodic(duration).await;
        assert!(result);
    }

    // Test for wait
    #[async_std::test]
    async fn test_wait() {
        let context = test_steady_context();
        let monitor = context.into_monitor([], []);

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
        let monitor = context.into_monitor([&rx], []);

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
        let monitor = context.into_monitor([&rx], []);

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
        let monitor = context.into_monitor([&rx], []);

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
        let mut monitor = context.into_monitor([], [&tx]);

        if let Some(mut tx) = tx.try_lock() {
            let result = monitor.send_async(&mut tx, 42, SendSaturation::Warn).await;
            assert!(result.is_ok());
        };
    }

    // Test for args method
    #[test]
    fn test_args() {
        let args = 42u32;
        let context = test_steady_context_with_args(args);
        let monitor = context.into_monitor([], []);

        let retrieved_args: Option<&u32> = monitor.args();
        assert_eq!(retrieved_args, Some(&42u32));
    }

    // Test for identity method
    #[test]
    fn test_identity() {
        let context = test_steady_context();
        let monitor = context.into_monitor( [], []);

        let identity = monitor.identity();
        assert_eq!(identity.label.name, "test_actor");
    }

    // Helper function to create a test context with arguments
    fn test_steady_context_with_args<A: Any + Send + Sync>(args: A) -> SteadyContext {
        let (_tx, rx) = build_tx_rx();
        SteadyContext {
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
            instance_id: 0,
            last_periodic_wait: Default::default(),
            is_in_graph: true,
            actor_start_time: Instant::now(),
            frame_rate_ms: 1000,
            team_id: 0,
            show_thread_info: false,
        }
    }
 
    // Test for is_liveliness_in
    #[test]
    fn test_is_liveliness_in() {
        let context = test_steady_context();
        let monitor = context.into_monitor( [], []);

        // Initially, the liveliness state should be Building
        assert!(monitor.is_liveliness_in(&[GraphLivelinessState::Building]));
        
    }

    // Test for yield_now
    #[async_std::test]
    async fn test_yield_now() {
        let context = test_steady_context();
        let monitor = context.into_monitor([], []);

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
        let monitor = context.into_monitor([&rx], []);

        if let Some(mut tx) = tx.try_lock() {
            tx.mark_closed();            
        }

        if let Some(mut rx) = rx.try_lock() {
         
            let result = monitor.wait_avail(&mut rx, 1).await;
            assert!(!result);
        };
    }

    // Test for wait_shutdown_or_vacant_units with shutdown requested
    #[async_std::test]
    async fn test_wait_shutdown_or_vacant_units_shutdown() {
        let (tx, _rx) = create_test_channel::<i32>(1);
        let context = test_steady_context();
        let tx = tx.clone();
        let monitor = context.into_monitor([], [&tx]);

        // Request shutdown
        {
            let mut liveliness = monitor.runtime_state.write();
            liveliness.request_shutdown();
        }

        if let Some(mut tx) = tx.try_lock() {
            let result = monitor.wait_vacant(&mut tx, 1).await;
            assert!(result);
        };
    }

    // Test for call_async with future that returns an error
    #[async_std::test]
    async fn test_call_async_with_error() {
        let context = test_steady_context();
        let monitor = context.into_monitor([], []);

        let fut = async { Err::<i32, &str>("error") };
        let result = monitor.call_async(fut).await;
        assert_eq!(result, Some(Err("error")));
    }
 
    // Test for args method with String
    #[test]
    fn test_args_string() {
        let args = "test_args".to_string();
        let context = test_steady_context_with_args(args.clone());
        let monitor = context.into_monitor([], []);

        let retrieved_args: Option<&String> = monitor.args();
        assert_eq!(retrieved_args, Some(&args));
    }



    // Test for take_slice with empty channel
    #[test]
    fn test_take_slice_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        let mut monitor = context.into_monitor( [&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.take_slice(&mut rx, &mut slice);
            assert_eq!(count, 0);
        };
    }

    // Test for send_slice_until_full with full channel
    #[test]
    fn test_send_slice_until_full_full_channel() {
        let (tx, _rx) = create_test_channel::<i32>(1);
        let context = test_steady_context();
        let tx = tx.clone();
        let mut monitor = context.into_monitor([], [&tx]);

        if let Some(mut tx) = tx.try_lock() {
            // Fill the channel
            let _ = monitor.try_send(&mut tx, 1);

            // Now the channel is full
            let slice = [2, 3, 4];
            let sent_count = monitor.send_slice_until_full(&mut tx, &slice);
            assert_eq!(sent_count, 0);
        };
    }

    // Test for take_into_iter with empty channel
    #[test]
    fn test_take_into_iter_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let mut monitor = context.into_monitor([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let mut iter = monitor.take_into_iter(&mut rx);
            assert_eq!(iter.next(), None);
        };
    }

    // Test for try_peek_slice with empty channel
    #[test]
    fn test_try_peek_slice_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        let monitor = context.into_monitor([&rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.try_peek_slice(&mut rx, &mut slice);
            assert_eq!(count, 0);
        };
    }

    // Test for try_take with empty channel
    #[test]
    fn test_try_take_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let mut monitor = context.into_monitor([&rx], []);

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
        let monitor = context.into_monitor([&rx], []);

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
        let mut monitor = context.into_monitor([], [&tx]);

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
        let monitor = context.into_monitor([&rx], []);

        if let Some(mut tx) = tx.try_lock() {
            tx.mark_closed();
        }
        if let Some(mut rx) = rx.try_lock() {         
            let result = monitor.wait_avail(&mut rx, 1).await;
            assert!(!result);
        };
    }






}
