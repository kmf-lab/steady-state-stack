use std::sync::Arc;
use futures::lock::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use async_ringbuf::AsyncRb;


pub(crate) type ChannelBacking<T> = Heap<T>;
pub(crate) type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;
pub(crate) type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;

//TODO: we want to use Static but will use heap as first step
//TODO: we must use static for all telemetry work.
//let mut rb = AsyncRb::<Static<T, 12>>::default().split_ref()
//   AsyncRb::<ChannelBacking<T>>::new(cap).split_ref()

use ringbuf::traits::Split;
use async_ringbuf::wrap::{AsyncCons, AsyncProd};
use log::info;
use ringbuf::storage::Heap;
use crate::steady_state::{DataType, Trigger, config, MONITOR_UNKNOWN, Rx, Tx};
use crate::steady_state::monitor::ChannelMetaData;

#[derive(Clone)]
pub struct ChannelBuilder {
    channel_count: Arc<AtomicUsize>,
    capacity: usize,
    labels: &'static [& 'static str],
    display_labels: bool,
    window_bucket_in_bits: u8, //ma is 1<<window_bucket_in_bits to ensure power of 2
    line_expansion: bool,
    show_type: bool,
    percentiles: Vec<DataType>, //each is a row
    std_dev: Vec<DataType>, //each is a row
    red: Vec<Trigger>, //if used base is green
    yellow: Vec<Trigger>, //if used base is green
    avg_inflight: bool,
    avg_consumed: bool,
}
//some ideas to target
// Primary Label - Latency Estimate (80th Percentile): This gives a quick, representative view of the channel's performance under load.
// Secondary Label - Moving Average of In-Flight Messages: This provides a sense of the current load on the channel.
// Tertiary Label (Optional) - Moving Average of Take Rate: This could be included if there's room and if the take rate is a critical performance factor for your application.


const DEFAULT_CAPACITY: usize = 64;

impl ChannelBuilder {


    pub(crate) fn new(channel_count: Arc<AtomicUsize>) -> ChannelBuilder {

        ChannelBuilder {
            channel_count,
            capacity: DEFAULT_CAPACITY,
            labels: &[],
            display_labels: false,
            window_bucket_in_bits: 5, // 1<<5 == 32
            line_expansion: false,
            show_type: false,
            percentiles: Vec::new(),
            std_dev: Vec::new(),
            red: Vec::new(),
            yellow: Vec::new(),
            avg_inflight: false,
            avg_consumed: false,
        }
    }

    /// moving average and percentile window will be at least this size but may be larger
    pub fn with_compute_window_floor(&self, duration: Duration) -> Self {
        let result = self.clone();

        let millis: u128 = duration.as_micros();
        let frame_ms: u128 = 1000u128 * config::TELEMETRY_PRODUCTION_RATE_MS as u128;
        let est_buckets = millis / frame_ms;
        //find the next largest power of 2
        let window_bucket_in_bits = (est_buckets as f32).log2().ceil() as u8;
        result.with_compute_window_bucket_bits(window_bucket_in_bits)
    }

    /// NOTE the default is 1 second
    pub fn with_compute_window_bucket_bits(&self, window_bucket_in_bits: u8) -> Self {
        assert!(window_bucket_in_bits <= 30);
        let mut result = self.clone();
        result.window_bucket_in_bits = window_bucket_in_bits;
        result

    }

    pub fn with_capacity(&self, capacity: usize) -> Self {
        let mut result = self.clone();
        result.capacity = capacity;
        result

    }

    /// show the type of T for this label
    pub fn with_type(&self) -> Self {
        let mut result = self.clone();
        result.show_type = true;
        result
    }

    /// expand the line weight as more data is processed over it
    pub fn with_line_expansion(&self) -> Self {
        let mut result = self.clone();
        result.line_expansion = true;
        result
    }

    pub fn with_avg_inflight(&self) -> Self {
        let mut result = self.clone();
        result.avg_inflight = true;
        result
    }

    pub fn with_avg_consumed(&self) -> Self {
        let mut result = self.clone();
        result.avg_consumed = true;
        result
    }

    /// show the control labels   TODO: dot filter must still be implemented
    pub fn with_labels(&self, labels: &'static [& 'static str], display: bool) -> Self {
        let mut result = self.clone();
        result.labels = labels;
        result
    }

    pub fn with_standard_deviation(&self, config: DataType) -> Self {
        let mut result = self.clone();
        result.std_dev.push(config);
        result
    }

    pub fn with_percentile(&self, config: DataType) -> Self {
        let mut result = self.clone();
        result.percentiles.push(config);
        result
    }

    pub fn with_red(&self, bound: Trigger) -> Self {
        let mut result = self.clone();
        result.red.push(bound);
        result
    }

    pub fn with_yellow(&self, bound: Trigger) -> Self {
        let mut result = self.clone();
        result.yellow.push(bound);
        result
    }

    pub fn to_meta_data(&self, type_name: &'static str) -> ChannelMetaData {

        let channel_id = self.channel_count.fetch_add(1, Ordering::SeqCst);

        ChannelMetaData {
            id: channel_id,
            labels: self.labels.into(),
            display_labels: self.display_labels,
            window_bucket_in_bits: self.window_bucket_in_bits,
            line_expansion: self.line_expansion,
            show_type: if self.show_type {Some(type_name.split("::").last().unwrap_or(""))} else {None},
            percentiles: self.percentiles.clone(),
            std_dev: self.std_dev.clone(),
            red: self.red.clone(),
            yellow: self.yellow.clone(),
            capacity: self.capacity,
            avg_inflight: self.avg_inflight,
            avg_consumed: self.avg_consumed,
        }
    }

    pub fn build<T>(& self) -> (Arc<Mutex<Tx<T>>>, Arc<Mutex<Rx<T>>>) {

        let rb = AsyncRb::<ChannelBacking<T>>::new(self.capacity);
        let (tx, rx) = rb.split();

        let channel_meta_data = Arc::new(self.to_meta_data(std::any::type_name::<T>()));

        let id = channel_meta_data.id;

        (  Arc::new(Mutex::new(Tx { id, tx
            , channel_meta_data: channel_meta_data.clone()
            , local_index: MONITOR_UNKNOWN }))
           , Arc::new(Mutex::new(Rx { id, rx
            , channel_meta_data: channel_meta_data.clone()
            , local_index: MONITOR_UNKNOWN }))
        )

    }

}

