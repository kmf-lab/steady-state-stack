
pub(crate) mod telemetry {
    pub(crate) mod metrics_collector;
    pub(crate) mod metrics_server;
    pub(crate) mod setup;
}

pub(crate) mod serialize {
    pub(crate) mod byte_buffer_packer;
    pub(crate) mod fast_protocol_packed;
}
pub(crate) mod stats;
pub(crate) mod config;
pub(crate) mod dot;
pub(crate) mod monitor;

pub mod channel;
pub mod util;
pub mod serviced;
pub mod graph;


use std::any::type_name;
use std::collections::HashMap;
//re-publish bastion from steady_state for this early version
pub use bastion;
use bastion::context::BastionContext;
use std::time::{Duration, Instant};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use futures::lock::Mutex;
use std::ops::{DerefMut, Sub};
use bastion::{Bastion, run};
use bastion::children::Children;
use std::future::{Future, ready};
use std::thread::sleep;
use log::error;
use channel::{InternalReceiver, InternalSender};
use ringbuf::producer::Producer;
use async_ringbuf::producer::AsyncProducer;
use ringbuf::traits::Observer;
use ringbuf::consumer::Consumer;
use async_ringbuf::consumer::AsyncConsumer;
use futures_timer::Delay;
use nuclei::config::{IoUringConfiguration, NucleiConfig};
use crate::steady_state::channel::ChannelBuilder;
use crate::steady_state::monitor::{ChannelMetaData, SteadyTelemetrySend};
use crate::steady_state::telemetry::metrics_collector::CollectorDetail;


use crate::steady_state::util::steady_logging_init;// re-publish in public

/// Initialize logging for the steady_state crate.
/// This is a convenience function that should be called at the beginning of main.
pub fn init_logging(loglevel: &str) -> Result<(), Box<dyn std::error::Error>> {

    //TODO: should probably be its own init function.
    init_nuclei();  //TODO: add some configs and a boolean to control this?

    steady_logging_init(loglevel)
}

#[derive(Clone)]
pub enum ChannelDataType {
    InFlight(f32),
    Consumed(f32),
}


/// Used when setting up new channels to specify when they should change to Red or Yellow state.
#[derive(Clone)]
pub enum ColorTrigger { //TODO: we need a vec of triggers not just one
    //pct filled channel capacity
    //
    //above x std dev of filled channel (0 is just above average fill)
    //
    //consume rate lower than x stand deviations (0 is just below average)
    //
    //latency above x stand deviations (0 is just above average)
    //
    //latency at X percentile above y milliseconds (SLA rule)
    //consume rate at X percentile above y milliseconds (SLA rule)
    //


    Percentile(u8),
    StdDev(f32),
    PercentFull(u8),
}
    // Primary Label - Latency Estimate (80th Percentile): This gives a quick, representative view of the channel's performance under load.
    //    Secondary Label - Moving Average of In-Flight Messages: This provides a sense of the current load on the channel.
    //   Tertiary Label (Optional) - Moving Average of Take Rate: This could be included if there's room and if the take rate is a critical performance factor for your application.


#[derive(PartialEq, Eq, Debug)]
pub enum GraphLivelinessState {
    Building,
    Running,
    StopRequested, //not yet confirmed by all nodes TODO: not yet implemented
    StopInProgress, //confirmed by all nodes and now stopping  TODO: not yet implemented
    Stopped,
}

pub struct SteadyContext {
    pub(crate) id: usize, //unique identifier for this child group
    pub(crate) name: & 'static str,
    pub(crate) ctx: Option<BastionContext>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<Mutex<GraphLivelinessState>>,
}

impl SteadyContext {

    pub fn id(self) -> usize {
        self.id
    }

    pub fn name(self) -> & 'static str {
        self.name
    }

    pub fn ctx(self) -> Option<BastionContext> {
        self.ctx
    }

    //method should be convert to_localmonitor
    pub fn into_monitor<const RX_LEN: usize, const TX_LEN: usize>(self
                                                      , rx_mons: &[& mut dyn RxDef; RX_LEN]
                                                      , tx_mons: &[& mut dyn TxDef; TX_LEN]
    ) -> LocalMonitor<RX_LEN,TX_LEN> {

        //only build telemetry channels if this feature is enabled
        let (telemetry_send_rx, telemetry_send_tx) = if config::TELEMETRY_HISTORY || config::TELEMETRY_SERVER {
             telemetry::setup::build_telemetry_channels(&self, rx_mons, tx_mons)
        } else {
            (None, None)
        };
         // this is my fixed size version for this specific thread
        LocalMonitor::<RX_LEN, TX_LEN> {
            telemetry_send_rx,
            telemetry_send_tx,
            last_instant: Instant::now().sub(Duration::from_secs(1+config::TELEMETRY_PRODUCTION_RATE_MS as u64)),
            id: self.id,
            name: self.name,
            ctx: self.ctx,
            runtime_state: self.runtime_state.clone(),
            #[cfg(test)]
            test_count: HashMap::new(),
        }
    }



}

pub struct Graph {
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) monitor_count: usize,
    //used by collector but could grow if we get new actors at runtime
    pub(crate) all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<Mutex<GraphLivelinessState>>,
}

//for testing only
#[cfg(test)]
impl Graph {

    /// needed for testing only, this monitor assumes we are running without a full graph
    pub fn new_test_monitor(self: &mut Self, name: & 'static str ) -> SteadyContext
    {
        let id = self.monitor_count;
        self.monitor_count += 1;

        let channel_count = self.channel_count.clone();
        let all_telemetry_rx = self.all_telemetry_rx.clone();
        SteadyContext {
            channel_count,
            name,
            ctx: None,
            id,
            all_telemetry_rx,
            runtime_state: self.runtime_state.clone()
        }
    }
}


impl Graph {

    pub fn start(&mut self) {
        Bastion::start(); //start the graph
        let mut guard = run!(self.runtime_state.lock());
        let state = guard.deref_mut();
        *state = GraphLivelinessState::Running;
    }

    pub fn request_shutdown(self: &mut Self) {
        let mut guard = run!(self.runtime_state.lock());
        let state = guard.deref_mut();
        *state = GraphLivelinessState::StopRequested;
    }

    /// add new children actors to the graph
    pub fn add_to_graph<F,I>(&mut self, name: & 'static str, c: Children, init: I ) -> Children
        where I: Fn(SteadyContext) -> F + Send + 'static + Clone,
              F: Future<Output = Result<(),()>> + Send + 'static ,  {
        graph::configure_for_graph(self, name, c, init)
    }

    /// create a new graph for the application typically done in main
    pub fn new() -> Graph {

        Graph {
            channel_count: Arc::new(AtomicUsize::new(0)),
            monitor_count: 0, //this is the count of all monitors
            all_telemetry_rx: Arc::new(Mutex::new(Vec::new())), //this is all telemetry receivers
            runtime_state: Arc::new(Mutex::new(GraphLivelinessState::Building))
        }
    }

    pub fn channel_builder(&mut self) -> ChannelBuilder {
        ChannelBuilder::new(self.channel_count.clone())
    }

    pub(crate) fn init_telemetry(&mut self) {
        telemetry::setup::build_optional_telemetry_graph(self);
    }
}


fn init_nuclei() {
    let nuclei_config = NucleiConfig {
        iouring: IoUringConfiguration::interrupt_driven(1 << 6),
        //iouring: IoUringConfiguration::kernel_poll_only(1 << 6),
        //iouring: IoUringConfiguration::low_latency_driven(1 << 6),
        //iouring: IoUringConfiguration::io_poll(1 << 6),
    };
    let _ = nuclei::Proactor::with_config(nuclei_config);

    nuclei::spawn_blocking(|| {
        // nuclei::drive(pending::<()>());
        nuclei::drive(async {
            loop {
                sleep(Duration::from_secs(10));
                ready(()).await;
            };
        });
    }).detach();
}

const MONITOR_UNKNOWN: usize = usize::MAX;
const MONITOR_NOT: usize = MONITOR_UNKNOWN-1; //any value below this is a valid monitor index

pub struct Tx<T> {
    pub(crate) id: usize,
    pub(crate) tx: InternalSender<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize,

}

pub struct Rx<T> {
    pub(crate) id: usize,
    pub(crate) rx: InternalReceiver<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize,
}


////////////////////////////////////////////////////////////////
impl<T> Tx<T> {
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
                error!("full channel detected tx labels:{:?} capacity:{:?} sending type:{} "
                , self.channel_meta_data.labels, self.tx.capacity(), type_name::<T>());
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




impl<T> Rx<T> {
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

pub trait TxDef {
    fn meta_data(&self) -> Arc<ChannelMetaData>;
}

impl <T> TxDef for Tx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        self.channel_meta_data.clone()
    }
}

pub trait RxDef {
    fn meta_data(&self) -> Arc<ChannelMetaData>;
}

impl <T> RxDef for Rx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        self.channel_meta_data.clone()
    }
}

pub struct LocalMonitor<const RX_LEN: usize, const TX_LEN: usize> {
    pub(crate) id: usize, //unique identifier for this child group
    pub(crate) name: & 'static str,
    pub(crate) ctx: Option<BastionContext>,
    pub(crate) telemetry_send_tx: Option<SteadyTelemetrySend<TX_LEN>>,
    pub(crate) telemetry_send_rx: Option<SteadyTelemetrySend<RX_LEN>>,
    pub(crate) last_instant:      Instant,
    pub(crate) runtime_state:     Arc<Mutex<GraphLivelinessState>>,
    #[cfg(test)]
    pub(crate) test_count: HashMap<&'static str, usize>,

}

///////////////////
impl <const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {

    pub(crate) fn id(&self) -> usize {
        self.id
    }

    pub(crate) fn name(&self) -> & 'static str {
        self.name
    }

    pub(crate) fn ctx(&self) -> Option<&BastionContext> {
        if let Some(ctx) = &self.ctx {
            Some(ctx)
        } else {
            None
        }
    }

    pub async fn relay_stats_all(&mut self) {
        telemetry::setup::send_all_local_telemetry(self).await;
    }

    pub async fn relay_stats_periodic(self: &mut Self, duration_rate: Duration) {
        assert!(duration_rate.ge(&Duration::from_micros(config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u64)));
        Delay::new(duration_rate.saturating_sub(self.last_instant.elapsed())).await;
        self.relay_stats_all().await;
    }

    pub fn take_slice<T>(& mut self, this: & mut Rx<T>, slice: &mut [T]) -> usize
        where T: Copy {
        let done = this.take_slice(slice);
        this.local_index = monitor::process_event(&mut self.telemetry_send_rx
                                                  , this.local_index, this.id,
                                                  |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(done));
        done
    }

    pub fn try_take<T>(& mut self, this: & mut Rx<T>) -> Option<T> {
        match this.try_take() {
            Some(msg) => {
                this.local_index = monitor::process_event(&mut self.telemetry_send_rx
                                                          , this.local_index, this.id,
                                                          |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(1));
                Some(msg)
            },
            None => {None}
        }
    }
    pub fn try_peek<'a,T>(&'a mut self, this: &'a mut Rx<T>) -> Option<&T> {
        this.try_peek()  //nothing to record since nothing moved. TODO: revisit this
    }
    pub fn is_empty<T>(& mut self, this: & mut Rx<T>) -> bool {
        this.is_empty()
    }

    pub fn avail_units<T>(& mut self, this: & mut Rx<T>) -> usize {
        this.avail_units()
    }

    pub async fn wait_avail_units<T>(& mut self, this: & mut Rx<T>, count:usize) {
        this.wait_avail_units(count).await
    }

    pub async fn peek_async<'a,T>(&'a mut self, this: &'a mut Rx<T>) -> Option<&T> {
        //nothing to record since nothing moved. TODO: revisit this
        this.peek_async().await

    }
    pub async fn take_async<T>(& mut self, this: & mut Rx<T>) -> Result<T, String> {
        match this.take_async().await {
            Ok(result) => {
                this.local_index = monitor::process_event(&mut self.telemetry_send_rx
                                                          , this.local_index, this.id,
                   |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(1));

                #[cfg(test)]
                self.test_count.entry("take_async").and_modify(|e| *e += 1).or_insert(1);

                Ok(result)
            },
            Err(error_msg) => {
                error!("Unexpected error take_async: {} {}", error_msg, self.name);
                Err(error_msg)
            }
        }
    }

    pub fn send_slice_until_full<T>(&mut self, this: & mut Tx<T>, slice: &[T]) -> usize
        where T: Copy {
        let done = this.send_slice_until_full(slice);
        this.local_index = monitor::process_event(&mut self.telemetry_send_tx
                                                  , this.local_index, this.id,
             |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(done));
        done
    }

    pub fn send_iter_until_full<T,I: Iterator<Item = T>>(&mut self, this: & mut Tx<T>, iter: I) -> usize {
        let done = this.send_iter_until_full(iter);
        this.local_index = monitor::process_event(&mut self.telemetry_send_tx
                                                  , this.local_index, this.id,
             |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(done));
        done
    }

    pub fn try_send<T>(& mut self, this: & mut Tx<T>, msg: T) -> Result<(), T> {
        match this.try_send(msg) {
            Ok(_) => {
                this.local_index = monitor::process_event(&mut self.telemetry_send_tx
                                                          , this.local_index, this.id,
                                                          |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(1));
                Ok(())
            },
            Err(sensitive) => {
                error!("Unexpected error try_send  telemetry: {} type: {}"
                    , self.name, type_name::<T>());
                Err(sensitive)
            }
        }
    }
    pub fn is_full<T>(& mut self, this: & mut Tx<T>) -> bool {
        this.is_full()
    }

    pub fn vacant_units<T>(& mut self, this: & mut Tx<T>) -> usize {
        this.vacant_units()
    }
    pub async fn wait_vacant_units<T>(& mut self, this: & mut Tx<T>, count:usize) {
        this.wait_vacant_units(count).await
    }

    pub async fn send_async<T>(& mut self, this: & mut Tx<T>, a: T) -> Result<(), T> {
       match this.send_async(a).await {
           Ok(_) => {
               this.local_index = monitor::process_event(&mut self.telemetry_send_tx
                                                         , this.local_index, this.id,
                    |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(1));
               Ok(())
           },
           Err(sensitive) => {
               error!("Unexpected error send_async  telemetry: {} type: {}", self.name, type_name::<T>());
               Err(sensitive)
           }
       }
    }




}


