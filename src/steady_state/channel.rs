use std::sync::Arc;
use futures::lock::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use async_ringbuf::AsyncRb;


pub(crate) type ChannelBacking<T> = Heap<T>;
pub(crate) type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;
pub(crate) type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;

//TODO: we want to use Static but will use heap as first step
//TODO: we must use static for all telemetry work.
//let mut rb = AsyncRb::<Static<T, 12>>::default().split_ref()
//   AsyncRb::<ChannelBacking<T>>::new(cap).split_ref()

use ringbuf::traits::Split;
use async_ringbuf::producer::AsyncProducer;
use async_ringbuf::wrap::{AsyncCons, AsyncProd};
use ringbuf::storage::Heap;
use crate::steady_state::{ColorTrigger, MONITOR_UNKNOWN, Rx, Tx};
use crate::steady_state::monitor::ChannelMetaData;

pub struct ChannelBuilder<T> {
    phantom: std::marker::PhantomData<T>,
    channel_counter: Arc<AtomicUsize>,
    capacity: usize,
    labels: &'static [& 'static str],
    display_labels: bool,
    window_in_seconds: u64, //for percentiles and ma
    line_expansion: bool,
    show_type: bool,
    percentiles: Vec<u8>, //each is a row
    std_dev: Vec<f32>, //each is a row
    red: Option<ColorTrigger>, //if used base is green
    yellow: Option<ColorTrigger>, //if used base is green

}

impl <T> ChannelBuilder<T> {
    pub(crate) fn new(channel_count: Arc<AtomicUsize>, capacity: usize) -> ChannelBuilder<T> {
        ChannelBuilder {
            phantom: std::marker::PhantomData,
            channel_counter: channel_count,
            capacity,
            labels: &[],
            display_labels: false,
            window_in_seconds: 1,
            line_expansion: false,
            show_type: false,
            percentiles: Vec::new(),
            std_dev: Vec::new(),
            red: None,
            yellow: None,
        }
    }



    /// NOTE the default is 1 second
    pub fn with_moving_average(self, window_in_seconds: u64) -> Self {
        assert!(window_in_seconds > 0);
        ChannelBuilder {
            phantom: std::marker::PhantomData,
            channel_counter: self.channel_counter,
            capacity: self.capacity,
            labels: self.labels,
            display_labels: self.display_labels,
            window_in_seconds,
            line_expansion: self.line_expansion,
            show_type: self.show_type,
            percentiles: self.percentiles,
            std_dev: self.std_dev,
            red: self.red,
            yellow: self.yellow,
        }
    }

    pub fn with_type(self) -> Self {
        ChannelBuilder {
            phantom: std::marker::PhantomData,
            channel_counter: self.channel_counter,
            capacity: self.capacity,
            labels: self.labels,
            display_labels: self.display_labels,
            window_in_seconds: self.window_in_seconds,
            line_expansion: self.line_expansion,
            show_type: true,
            percentiles: self.percentiles,
            std_dev: self.std_dev,
            red: self.red,
            yellow: self.yellow,
        }
    }

    pub fn with_line_expansion(self) -> Self {
        ChannelBuilder {
            phantom: std::marker::PhantomData,
            channel_counter: self.channel_counter,
            capacity: self.capacity,
            labels: self.labels,
            display_labels: self.display_labels,
            window_in_seconds: self.window_in_seconds,
            line_expansion: true,
            show_type: self.show_type,
            percentiles: self.percentiles,
            std_dev: self.std_dev,
            red: self.red,
            yellow: self.yellow,
        }
    }

    pub fn with_red(self, bound: ColorTrigger) -> Self {
        ChannelBuilder {
            phantom: std::marker::PhantomData,
            channel_counter: self.channel_counter,
            capacity: self.capacity,
            labels: self.labels,
            display_labels: self.display_labels,
            window_in_seconds: self.window_in_seconds,
            line_expansion: true,
            show_type: self.show_type,
            percentiles: self.percentiles,
            std_dev: self.std_dev,
            red: Some(bound),
            yellow: self.yellow,
        }
    }

    pub fn with_yellow(self, bound: ColorTrigger) -> Self {
        ChannelBuilder {
            phantom: std::marker::PhantomData,
            channel_counter: self.channel_counter,
            capacity: self.capacity,
            labels: self.labels,
            display_labels: self.display_labels,
            window_in_seconds: self.window_in_seconds,
            line_expansion: true,
            show_type: self.show_type,
            percentiles: self.percentiles,
            std_dev: self.std_dev,
            red: self.red,
            yellow: Some(bound),
        }
    }


    pub fn with_percentile(self, percentile: u8) -> Self {
        let mut v = self.percentiles;
        v.push(percentile);

        ChannelBuilder {
            phantom: std::marker::PhantomData,
            channel_counter: self.channel_counter,
            capacity: self.capacity,
            labels: self.labels,
            display_labels: self.display_labels,
            window_in_seconds: self.window_in_seconds,
            line_expansion: self.line_expansion,
            show_type: self.show_type,
            percentiles: v,
            std_dev: self.std_dev,
            red: self.red,
            yellow: self.yellow,
        }
    }

    pub fn with_standard_deviation(self, show_std_dev: f32) -> Self {
        let mut v = self.std_dev;
        v.push(show_std_dev);

        ChannelBuilder {
            phantom: std::marker::PhantomData,
            channel_counter: self.channel_counter,
            capacity: self.capacity,
            labels: self.labels,
            display_labels: self.display_labels,
            window_in_seconds: self.window_in_seconds,
            line_expansion: self.line_expansion,
            show_type: self.show_type,
            percentiles: self.percentiles,
            std_dev: v,
            red: self.red,
            yellow: self.yellow,
        }
    }

    pub fn with_labels(self, labels: &'static [& 'static str], display: bool) -> Self {
        ChannelBuilder {
            phantom: std::marker::PhantomData,
            channel_counter: self.channel_counter,
            capacity: self.capacity,
            labels,
            display_labels: display,
            window_in_seconds: self.window_in_seconds,
            line_expansion: self.line_expansion,
            show_type: self.show_type,
            percentiles: self.percentiles,
            std_dev: self.std_dev,
            red: self.red,
            yellow: self.yellow,
        }
    }

    pub fn to_meta_data(&self, id:usize) -> ChannelMetaData {
        ChannelMetaData {
            id,
            labels: self.labels.into(),
            display_labels: self.display_labels,
            window_in_seconds: self.window_in_seconds,
            line_expansion: self.line_expansion,
            show_type: self.show_type,
            percentiles: self.percentiles.clone(),
            std_dev: self.std_dev.clone(),
            red: self.red.clone(),
            yellow: self.yellow.clone(),
        }
    }

    pub fn build(self) -> (Arc<Mutex<Tx<T>>>, Arc<Mutex<Rx<T>>>) {
        self.new_channel()
    }

    fn new_channel(self) -> (Arc<Mutex<Tx<T>>>, Arc<Mutex<Rx<T>>>) {
        let channel_id = self.channel_counter.fetch_add(1, Ordering::SeqCst);
        let capacity = self.capacity;
        let rb = AsyncRb::<ChannelBacking<T>>::new(capacity);
        let (tx, rx) = rb.split();

        let channel_meta_data = Arc::new(self.to_meta_data(channel_id));
        (  Arc::new(Mutex::new(Tx { id: channel_id
                         , batch_limit: capacity
                         , tx
                         , channel_meta_data: channel_meta_data.clone()
                         , local_index: MONITOR_UNKNOWN }))
         , Arc::new(Mutex::new(Rx { id: channel_id
                         , batch_limit: capacity
                         , rx
                         , channel_meta_data: channel_meta_data.clone()
                         , local_index: MONITOR_UNKNOWN }))
        )


    }
}

