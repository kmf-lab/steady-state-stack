use std::any::type_name;
use std::sync::Arc;
use futures::lock::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use async_ringbuf::AsyncRb;


type ChannelBacking<T> = Heap<T>;
type InternalSender<T> = AsyncProd<Arc<AsyncRb<ChannelBacking<T>>>>;
type InternalReceiver<T> = AsyncCons<Arc<AsyncRb<ChannelBacking<T>>>>;

//TODO: we want to use Static but will use heap as first step
//TODO: we must use static for all telemetry work.
//let mut rb = AsyncRb::<Static<T, 12>>::default().split_ref()
//   AsyncRb::<ChannelBacking<T>>::new(cap).split_ref()

use ringbuf::traits::{Observer, Split};
use ringbuf::consumer::Consumer;
use async_ringbuf::consumer::AsyncConsumer;
use log::error;
use ringbuf::producer::Producer;
use async_ringbuf::producer::AsyncProducer;
use async_ringbuf::wrap::{AsyncCons, AsyncProd};
use ringbuf::storage::Heap;
use crate::steady::monitor::ChannelMetaData;

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
    red: Option<ChannelBound>, //if used base is green
    yellow: Option<ChannelBound>, //if used base is green

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

    pub fn with_red(self, bound: ChannelBound) -> Self {
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

    pub fn with_yellow(self, bound: ChannelBound) -> Self {
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

    pub fn build(self) -> (Arc<Mutex<SteadyTx<T>>>, Arc<Mutex<SteadyRx<T>>>) {
        self.new_channel()
    }

    fn new_channel(self) -> (Arc<Mutex<SteadyTx<T>>>, Arc<Mutex<SteadyRx<T>>>) {
        let channel_id = self.channel_counter.fetch_add(1, Ordering::SeqCst);
        let capacity = self.capacity;
        let rb = AsyncRb::<ChannelBacking<T>>::new(capacity);
        let (tx, rx) = rb.split();

        let channel_meta_data = Arc::new(self.to_meta_data(channel_id));
        (  Arc::new(Mutex::new(SteadyTx { id: channel_id
                         , actor_name: None
                         , batch_limit: capacity
                         , tx
                         , channel_meta_data: channel_meta_data.clone()
                         , local_idx: usize::MAX }))
         , Arc::new(Mutex::new(SteadyRx { id: channel_id
                         , batch_limit: capacity
                         , rx
                         , channel_meta_data: channel_meta_data.clone()
                         , local_idx: usize::MAX  }))
        )


    }
}

#[derive(Clone)]
pub enum ChannelBound {
    Percentile(u8),
    StdDev(f32),
    PercentFull(u8),
}


pub struct SteadyRx<T> {
    pub(crate) id: usize,
    pub(crate) batch_limit: usize,
    pub(crate) rx: InternalReceiver<T>,
    pub(crate) local_idx: usize,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
}


impl<T> SteadyRx<T> {
    #[inline]
    pub async fn take_async(& mut self) -> Result<T,String> {
        // implementation favors a full channel
        if let Some(m) = self.rx.try_pop() {
            Ok(m)
        } else {
            match self.rx.pop().await {
                Some(a) => { Ok(a) }
                None => { Err("Producer is dropped".to_string()) }
            }
        }
    }

    #[inline]
    pub fn try_take(& mut self) -> Option<T> {
        self.rx.try_pop()
    }

    //removes items from the channel and Copy's them into the slice
    //returns the number of items removed
    #[inline]
    pub fn take_slice(&mut self, elems: &mut [T]) -> usize
        where T: Copy {
           self.rx.pop_slice(elems)
    }

    #[inline]
    pub fn try_peek(&self) -> Option<&T> {
        self.rx.first()
    }

    #[inline]
    pub async fn peek_async(& mut self) -> Option<&T> {
        self.rx.wait_occupied(1).await;
        self.rx.first()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
    #[inline]
    pub fn avail_units(& mut self) -> usize {
        self.rx.occupied_len()
    }

    #[inline]
    pub async fn wait_avail_units(& mut self, count: usize) {
        self.rx.wait_occupied(count).await
    }

}


pub struct SteadyTx<T> {
    pub(crate) id: usize,
    pub(crate) batch_limit: usize,
    pub(crate) tx: InternalSender<T>,
    pub(crate) local_idx: usize,
    pub(crate) actor_name: Option<&'static str>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,


}


////////////////////////////////////////////////////////////////
impl<T> SteadyTx<T> {
    #[inline]
    pub fn try_send(& mut self, msg: T) -> Result<(), T> {
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())}
            Err(m) => {Err(m)}
        }
    }

    #[inline]
    pub fn send_iter_until_full<I: Iterator<Item = T>>(&mut self, iter: I) -> usize {
        self.tx.push_iter(iter)
    }

    #[inline]
    pub fn send_slice_until_full(&mut self, slice: &[T]) -> usize
       where T: Copy {
        self.tx.push_slice(slice)
    }

    #[inline]
    pub async fn send_async(& mut self, msg: T) -> Result<(), T> {
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())},
            Err(msg) => {
                error!("full channel detected tx actor:{:?} labels:{:?} capacity:{:?} sending type:{} "
                , self.actor_name, self.channel_meta_data.labels, self.tx.capacity(), type_name::<T>());
                //here we will await until there is room in the channel

                self.tx.push(msg).await
            }
        }
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.tx.is_full()
    }

    #[inline]
    pub fn vacant_units(&self) -> usize {
        self.tx.vacant_len()
    }

    #[inline]
    pub async fn wait_vacant_units(&self, count: usize) {
        self.tx.wait_vacant(count).await
    }


}

