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
#[allow(unused_imports)]
use log::*;
use ringbuf::storage::Heap;
use crate::{AlertColor, config, Filled, MONITOR_UNKNOWN, Percentile, Rate, Rx, StdDev, SteadyRx, SteadyTx, Trigger, Tx};
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

    pub fn with_avg_filled(&self) -> Self {
        let mut result = self.clone();
        result.avg_filled = true;
        result
    }

    pub fn with_avg_rate(&self) -> Self {
        let mut result = self.clone();
        result.avg_rate = true;
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
        result.std_dev_filled.push(config);
        result
    }
    pub fn with_rate_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_rate.push(config);
        result
    }
    pub fn with_latency_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_latency.push(config);
        result
    }


    pub fn with_filled_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_filled.push(config);
        result
    }
    pub fn with_rate_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_rate.push(config);
        result
    }
    pub fn with_latency_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_latency.push(config);
        result
    }

    pub fn with_rate_trigger(&self, bound: Trigger<Rate>, color: AlertColor) -> Self {
        let mut result = self.clone();
        //need sequence number for these triggers?
        result.trigger_rate.push((bound, color));
        result
    }
    pub fn with_filled_trigger(&self, bound: Trigger<Filled>, color: AlertColor) -> Self {
        let mut result = self.clone();
        //need sequence number for these triggers?
        result.trigger_filled.push((bound, color));
        result
    }
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
            percentiles_inflight: self.percentiles_filled.clone(),
            percentiles_consumed: self.percentiles_rate.clone(),
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

    pub fn build<T>(& self) -> (SteadyTx<T>, SteadyRx<T>) {

        let rb = AsyncRb::<ChannelBacking<T>>::new(self.capacity);
        let (tx, rx) = rb.split();

        //trace!("channel_builder::build: type_name: {}", std::any::type_name::<T>());

        //the number of bytes consumed by T
        let type_byte_count = std::mem::size_of::<T>();
        let type_string_name = std::any::type_name::<T>();
        let channel_meta_data = Arc::new(self.to_meta_data(type_string_name,type_byte_count));
        let id = channel_meta_data.id;

        (  Arc::new(Mutex::new(Tx { id, tx
            , channel_meta_data: channel_meta_data.clone()
            , local_index: MONITOR_UNKNOWN
            , last_error_send: Instant::now() //TODO: roll back a few seconds..
        }))
           , Arc::new(Mutex::new(Rx { id, rx
            , channel_meta_data
            , local_index: MONITOR_UNKNOWN }))
        )

    }

}

