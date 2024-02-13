use std::sync::Arc;
use futures::lock::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use async_ringbuf::AsyncRb;


pub(crate) type ChannelBacking<T> = Heap<T>;
pub(crate) type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;
pub(crate) type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;

pub type SteadyTx<T> = Arc<Mutex<Tx<T>>>;
pub type SteadyRx<T> = Arc<Mutex<Rx<T>>>;




//TODO: we should use static for all telemetry work. (next step)
//      this might lead to a general solution for static in other places
//let mut rb = AsyncRb::<Static<T, 12>>::default().split_ref()
//   AsyncRb::<ChannelBacking<T>>::new(cap).split_ref()

use ringbuf::traits::Split;
use async_ringbuf::wrap::{AsyncCons, AsyncProd};
#[allow(unused_imports)]
use log::*;
use ringbuf::storage::Heap;
use crate::{config, MONITOR_UNKNOWN, Percentile, Rx, StdDev, Trigger, Tx};
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
    percentiles_inflight: Vec<Percentile>, //each is a row
    percentiles_consumed: Vec<Percentile>, //each is a row
    percentiles_latency: Vec<Percentile>, //each is a row
    std_dev_inflight: Vec<StdDev>, //each is a row
    std_dev_consumed: Vec<StdDev>, //each is a row
    std_dev_latency: Vec<StdDev>, //each is a row
    red: Vec<Trigger>, //if used base is green
    yellow: Vec<Trigger>, //if used base is green
    avg_inflight: bool,
    avg_consumed: bool,
    avg_latency: bool,
    connects_sidecar: bool,
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

            refresh_rate_in_bits: 6, // 1<<6 == 64
            window_bucket_in_bits: 5, // 1<<5 == 32

            line_expansion: false,
            show_type: false,
            percentiles_inflight: Vec::new(),
            percentiles_consumed: Vec::new(),
            percentiles_latency: Vec::new(),
            std_dev_inflight: Vec::new(),
            std_dev_consumed: Vec::new(),
            std_dev_latency: Vec::new(),
            red: Vec::new(),
            yellow: Vec::new(),
            avg_inflight: false,
            avg_consumed: false,
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
        let window_in_bits = (buckets_per_window as f32).log2().ceil() as u8;
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

    pub fn with_avg_latency(&self) -> Self {
        let mut result = self.clone();
        result.avg_latency = true;
        result
    }



    pub fn connects_sidecar(&self) -> Self {
        let mut result = self.clone();
        result.connects_sidecar = true;
        result
    }


    pub fn with_labels(&self, labels: &'static [& 'static str], display: bool) -> Self {
        let mut result = self.clone();
        result.labels = if display {labels} else {&[]};
        result
    }


    pub fn with_filled_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_inflight.push(config);
        result
    }
    pub fn with_rate_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_consumed.push(config);
        result
    }
    pub fn with_latency_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_latency.push(config);
        result
    }


    pub fn with_filled_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_inflight.push(config);
        result
    }
    pub fn with_rate_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_consumed.push(config);
        result
    }
    pub fn with_latency_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_latency.push(config);
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

    pub(crate) fn to_meta_data(&self, type_name: &'static str) -> ChannelMetaData {
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
            show_type,
            percentiles_inflight: self.percentiles_inflight.clone(),
            percentiles_consumed: self.percentiles_consumed.clone(),
            percentiles_latency: self.percentiles_latency.clone(),
            std_dev_inflight: self.std_dev_inflight.clone(),
            std_dev_consumed: self.std_dev_consumed.clone(),
            std_dev_latency: vec![],
            red: self.red.clone(),
            yellow: self.yellow.clone(),
            capacity: self.capacity,
            avg_inflight: self.avg_inflight,
            avg_consumed: self.avg_consumed,
            avg_latency: false,
            connects_sidecar: self.connects_sidecar,
        }
    }

    pub fn build<T>(& self) -> (SteadyTx<T>, SteadyRx<T>) {

        let rb = AsyncRb::<ChannelBacking<T>>::new(self.capacity);
        let (tx, rx) = rb.split();

        //trace!("channel_builder::build: type_name: {}", std::any::type_name::<T>());

        let channel_meta_data = Arc::new(self.to_meta_data(std::any::type_name::<T>()));
        let id = channel_meta_data.id;

        (  Arc::new(Mutex::new(Tx { id, tx
            , channel_meta_data: channel_meta_data.clone()
            , local_index: MONITOR_UNKNOWN }))
           , Arc::new(Mutex::new(Rx { id, rx
            , channel_meta_data
            , local_index: MONITOR_UNKNOWN }))
        )

    }

}

