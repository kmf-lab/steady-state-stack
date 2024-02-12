
pub(crate) mod telemetry {
    pub(crate) mod metrics_collector;
    pub(crate) mod metrics_server;
    pub(crate) mod setup;
}

pub(crate) mod serialize {
    pub(crate) mod byte_buffer_packer;
    pub(crate) mod fast_protocol_packed;
}
pub(crate) mod channel_stats;
pub(crate) mod config;
pub(crate) mod dot;
pub(crate) mod monitor;

pub mod channel_builder;
pub mod util;
pub mod serviced;
pub mod actor_builder;
mod actor_stats;


use std::any::{Any, type_name};
#[cfg(test)]
use std::collections::HashMap;
//re-publish bastion from steady_state for this early version
pub use bastion;
use bastion::context::BastionContext;
use std::time::{Duration, Instant};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize};
use futures::lock::Mutex;
use std::ops::{DerefMut, Sub};
use bastion::{Bastion, run};
use std::future::{Future, ready};
use std::thread::sleep;
use log::error;
use channel_builder::{InternalReceiver, InternalSender};
use ringbuf::producer::Producer;
use async_ringbuf::producer::AsyncProducer;
use ringbuf::traits::Observer;
use ringbuf::consumer::Consumer;
use async_ringbuf::consumer::AsyncConsumer;
use futures_timer::Delay;
use nuclei::config::{IoUringConfiguration, NucleiConfig};
use actor_builder::ActorBuilder;
use crate::channel_builder::ChannelBuilder;
use crate::monitor::{ChannelMetaData, SteadyTelemetryActorSend, SteadyTelemetrySend};
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::util::steady_logging_init;// re-publish in public

/// Initialize logging for the steady_state crate.
/// This is a convenience function that should be called at the beginning of main.
pub fn init_logging(loglevel: &str) -> Result<(), Box<dyn std::error::Error>> {

    //TODO: should probably be its own init function.
    init_nuclei();  //TODO: add some configs and a boolean to control this?

    steady_logging_init(loglevel)
}



/// Used when setting up new channels to specify when they should change to Red or Yellow state.
#[derive(Clone)]
pub enum Trigger {
    AvgFilledAbove(Filled),
    AvgFilledBelow(Filled),
    StdDevsFilledAbove(StdDev, Filled), // above mean+(std*factor)
    StdDevsFilledBelow(StdDev, Filled), // below mean-(std*factor)
    PercentileFilledAbove(Percentile, Filled),
    PercentileFilledBelow(Percentile, Filled),
    /////////////////////////////////////////////

    AvgRateBelow(Rate),
    AvgRateAbove(Rate),
    StdDevRateBelow(StdDev,Rate), // below mean-(std*factor)
    StdDevRateAbove(StdDev,Rate), // above mean+(std*factor)
    PercentileRateAbove(Percentile, Rate),
    PercentileRateBelow(Percentile, Rate),

    //////////////////////////

    AvgLatencyAbove(Duration),
    AvgLatencyBelow(Duration),
    StdDevLatencyAbove(StdDev,Duration), // above mean+(std*factor)
    StdDevLatencyBelow(StdDev,Duration), // below mean-(std*factor)
    PercentileLatencyAbove(Percentile, Duration),
    PercentileLatencyBelow(Percentile, Duration), //not sure if this is useful

}


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
    pub(crate) redundancy: usize,
    pub(crate) name: & 'static str,
    pub(crate) ctx: Option<BastionContext>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<Mutex<GraphLivelinessState>>,
    pub(crate) count_restarts: Arc<AtomicU32>,
    pub(crate) args: Arc<Box<dyn Any+Send+Sync>>,
}

impl SteadyContext {

    pub fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn name(&self) -> & 'static str {
        self.name
    }

    pub fn ctx(&self) -> &Option<BastionContext> {
        &self.ctx
    }


    pub fn into_monitor<const RX_LEN: usize, const TX_LEN: usize>(self
                                                      , rx_mons: &[& mut dyn RxDef; RX_LEN]
                                                      , tx_mons: &[& mut dyn TxDef; TX_LEN]
    ) -> LocalMonitor<RX_LEN,TX_LEN> {

        //only build telemetry channels if this feature is enabled
        let (telemetry_send_rx, telemetry_send_tx, telemetry_state) = if config::TELEMETRY_HISTORY || config::TELEMETRY_SERVER {
             telemetry::setup::build_telemetry_channels(&self, rx_mons, tx_mons)
        } else {
            (None, None, None)
        };

        let start_instant = Instant::now().sub(Duration::from_secs(1+config::TELEMETRY_PRODUCTION_RATE_MS as u64));


        // this is my fixed size version for this specific thread
        LocalMonitor::<RX_LEN, TX_LEN> {
            telemetry_send_rx,
            telemetry_send_tx,
            telemetry_state,
            last_telemetry_send: start_instant,
            id: self.id,
            name: self.name,
            ctx: self.ctx,
            runtime_state: self.runtime_state.clone(),
            #[cfg(test)]
            test_count: HashMap::new(),
        }
    }



}

pub struct Graph  {
    pub(crate) args: Arc<Box<dyn Any+Send+Sync>>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) monitor_count: usize,
    //used by collector but could grow if we get new actors at runtime
    pub(crate) all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<Mutex<GraphLivelinessState>>,
}

impl Graph {

    /// needed for testing only, this monitor assumes we are running without a full graph
    /// and will not be used in production
    pub fn new_test_monitor(&mut self, name: & 'static str ) -> SteadyContext
    {
        // assert that we are NOT in release mode
        assert!(cfg!(debug_assertions), "This function is only for testing");

        let id = self.monitor_count;
        self.monitor_count += 1;

        let channel_count    = self.channel_count.clone();
        let all_telemetry_rx = self.all_telemetry_rx.clone();

        let count_restarts   = Arc::new(AtomicU32::new(0));
        SteadyContext {
            channel_count,
            name,
            args: self.args.clone(),
            ctx: None, //this is key, we are not running in a graph by design
            id,
            redundancy: 1,
            all_telemetry_rx,
            runtime_state: self.runtime_state.clone(),
            count_restarts,
        }
    }
}


impl Graph {

    pub fn actor_builder(&mut self, name: & 'static str) -> ActorBuilder{
        crate::ActorBuilder::new(self, name)
    }

    pub fn start(&mut self) {

        //TODO: move for special debug flag.
        /*
        #[cfg(debug_assertions)]
        std::panic::set_hook(Box::new(|panic_info| {
            let backtrace = Backtrace::capture();

            // You can log the panic information here if needed
            eprintln!("Application panicked: {}", panic_info);

            eprintln!("Backtrace:\n{:?}", backtrace);

            // Exit with status code -1
            exit(-1);
        }));
        //  */

        Bastion::start(); //start the graph
        let mut guard = run!(self.runtime_state.lock());
        let state = guard.deref_mut();
        *state = GraphLivelinessState::Running;
    }

    pub fn request_shutdown(&mut self) {
        let mut guard = run!(self.runtime_state.lock());
        let state = guard.deref_mut();
        *state = GraphLivelinessState::StopRequested;
    }

    /// create a new graph for the application typically done in main
    pub fn new<A: Any+Send+Sync>(args: A) -> Graph {
        Graph {
            args: Arc::new(Box::new(args)),
            channel_count: Arc::new(AtomicUsize::new(0)),
            monitor_count: 0, //this is the count of all monitors
            all_telemetry_rx: Arc::new(Mutex::new(Vec::new())), //this is all telemetry receivers
            runtime_state: Arc::new(Mutex::new(GraphLivelinessState::Building))
        }
    }

    pub fn channel_builder(&mut self) -> ChannelBuilder {
        ChannelBuilder::new(self.channel_count.clone())
    }

    pub fn init_telemetry(&mut self) {
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

    #[inline]
    pub async fn wait_empty(&self) {
        self.tx.wait_vacant(usize::from(self.tx.capacity())).await
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
    pub fn try_peek_vec<'a>(&'a self, target:&'a mut Vec<&'a T>) {
        target.clear();
        target.extend(self.rx.iter());
    }

    #[inline]
    pub fn try_peek_iter(& self) -> impl Iterator<Item = & T>  {
        self.rx.iter()
    }


    #[inline]
    pub async fn peek_async(& mut self) -> Option<&T> {
        self.rx.wait_occupied(1).await;
        self.rx.first()
    }

    #[inline]
    pub async fn peek_async_vec<'a>(&'a mut self, wait_for_count: usize, target:&'a mut Vec<&'a T>) -> &mut Vec<&T> {
        target.clear();
        self.rx.wait_occupied(wait_for_count).await;
        target.extend(self.rx.iter());
        target
    }

    pub async fn peek_async_iter(& mut self, wait_for_count: usize) -> impl Iterator<Item = & T> {
        self.rx.wait_occupied(wait_for_count).await;
        self.rx.iter()
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
pub trait RxDef {
    fn meta_data(&self) -> Arc<ChannelMetaData>;
}
impl <T> TxDef for Tx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        self.channel_meta_data.clone()
    }
}
impl <T> RxDef for Rx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        self.channel_meta_data.clone()
    }
}

impl <const RXL: usize, const TXL: usize> Drop for LocalMonitor<RXL, TXL> {
    //if possible we never want to loose telemetry data so we try to flush it out
    fn drop(&mut self) {
        run!(
            telemetry::setup::send_all_local_telemetry_async(self)
          );
    }

}

pub struct LocalMonitor<const RX_LEN: usize, const TX_LEN: usize> {
    pub(crate) id:                   usize, //unique identifier for this child group
    pub(crate) name:                 & 'static str,
    pub(crate) ctx:                  Option<BastionContext>,
    pub(crate) telemetry_send_tx:    Option<SteadyTelemetrySend<TX_LEN>>,
    pub(crate) telemetry_send_rx:    Option<SteadyTelemetrySend<RX_LEN>>,
    pub(crate) telemetry_state:      Option<SteadyTelemetryActorSend>,
    pub(crate) last_telemetry_send:  Instant,
    pub(crate) runtime_state:        Arc<Mutex<GraphLivelinessState>>,


    #[cfg(test)]
    pub(crate) test_count: HashMap<&'static str, usize>,

}

///////////////////
impl <const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn name(&self) -> & 'static str {
        self.name
    }

    pub fn runtime_state(&self) -> Arc<Mutex<GraphLivelinessState>> {
        self.runtime_state.clone()
    }

    pub fn ctx(&self) -> Option<&BastionContext> {
        if let Some(ctx) = &self.ctx {
            Some(ctx)
        } else {
            None
        }
    }


    pub async fn stop(&mut self) -> Result<(),()>  {
        if let Some(ref mut st) = self.telemetry_state {
            st.bool_stop = true;
        }// upon drop we will flush telemetry
        Ok(())
    }

    pub async fn relay_stats_all(&mut self) {
        //NOTE: not time ing this one as it is mine and internal
        telemetry::setup::try_send_all_local_telemetry(self).await;
    }

    pub async fn relay_stats_periodic(&mut self, duration_rate: Duration) {

        self.start_hot_profile(monitor::CALL_WAIT);

        assert!(duration_rate.ge(&Duration::from_micros(config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u64)));
        Delay::new(duration_rate.saturating_sub(self.last_telemetry_send.elapsed())).await;
        self.rollup_hot_profile();

        //this can not be measured since it sends the measurement of hot_profile.
        //also this is a special case where we do not want to measure the time it takes to send telemetry
        self.relay_stats_all().await;


    }

    fn start_hot_profile(&mut self, x: usize) {
        if let Some(ref mut st) = self.telemetry_state {
            st.calls[x] = st.calls[x].saturating_add(1);
            if st.hot_profile.is_none() {
                st.hot_profile = Some(Instant::now())
            }
        };
    }

    pub fn take_slice<T>(&mut self, this: & mut Rx<T>, slice: &mut [T]) -> usize
        where T: Copy {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_BATCH_READ]=st.calls[monitor::CALL_BATCH_READ].saturating_add(1);
        }

        let done = this.take_slice(slice);
        this.local_index = monitor::process_event(&mut self.telemetry_send_rx
                                                  , this.local_index, this.id,
                                                  |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(done));


        done
    }

    pub fn try_take<T>(&mut self, this: & mut Rx<T>) -> Option<T> {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_SINGLE_READ]=st.calls[monitor::CALL_SINGLE_READ].saturating_add(1);
        }

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
        this.try_peek()
    }

    //TODO:  slice not vec.
    pub fn try_peek_populate_vec<'a,T>(&'a mut self, this: &'a mut Rx<T>, target:&'a mut Vec<&'a T>) {
        this.try_peek_vec(target);
    }

    pub fn try_peek_iter<'a,T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a {
       this.try_peek_iter()
    }

    pub async fn peek_async_populate_vec<'a,T>(&'a mut self, this: &'a mut Rx<T>, wait_for_count: usize, target:&'a mut Vec<&'a T>) -> &mut Vec<&T> {

        self.start_hot_profile(monitor::CALL_OTHER);

        let result = this.peek_async_vec(wait_for_count,target).await;

        self.rollup_hot_profile();


        result

    }

    pub async fn peek_async_iter<'a,T>(&'a mut self, this: &'a mut Rx<T>, wait_for_count: usize) -> impl Iterator<Item = &'a T> + 'a {

        self.start_hot_profile(monitor::CALL_OTHER);

        let result = this.peek_async_iter(wait_for_count).await;

        self.rollup_hot_profile();

        result

    }

    pub fn is_empty<T>(& mut self, this: & mut Rx<T>) -> bool {
        this.is_empty()
    }

    pub fn avail_units<T>(& mut self, this: & mut Rx<T>) -> usize {
        this.avail_units()
    }

    pub async fn wait(& mut self, duration: Duration) {

        self.start_hot_profile(monitor::CALL_WAIT);

        Delay::new(duration).await;

        self.rollup_hot_profile();


    }

    //here we just take an async fn and call it async just to wrap it
    pub async fn call_async<F>(&mut self, f: F) -> F::Output
        where F: Future {

        self.start_hot_profile(monitor::CALL_OTHER);

        let result = f.await;

        self.rollup_hot_profile();


        result
    }


    pub async fn wait_avail_units<T>(&mut self, this: & mut Rx<T>, count:usize) {

        self.start_hot_profile(monitor::CALL_OTHER);

        let result = this.wait_avail_units(count).await;

        self.rollup_hot_profile();


        result


    }

    pub async fn peek_async<'a,T>(&'a mut self, this: &'a mut Rx<T>) -> Option<&T> {

        self.start_hot_profile(monitor::CALL_OTHER);

        let result = this.peek_async().await;

        self.rollup_hot_profile();


        result
    }
    pub async fn take_async<T>(& mut self, this: & mut Rx<T>) -> Result<T, String> {

        self.start_hot_profile(monitor::CALL_SINGLE_READ);

        let result = this.take_async().await;

        self.rollup_hot_profile();


        match result {
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

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_BATCH_WRITE]=st.calls[monitor::CALL_BATCH_WRITE].saturating_add(1);
        }

        let done = this.send_slice_until_full(slice);
        this.local_index = monitor::process_event(&mut self.telemetry_send_tx
                                                  , this.local_index, this.id,
             |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(done));
        done
    }

    pub fn send_iter_until_full<T,I: Iterator<Item = T>>(&mut self, this: & mut Tx<T>, iter: I) -> usize {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_BATCH_WRITE]=st.calls[monitor::CALL_BATCH_WRITE].saturating_add(1);
        }

        let done = this.send_iter_until_full(iter);
        this.local_index = monitor::process_event(&mut self.telemetry_send_tx
                                                  , this.local_index, this.id,
             |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(done));
        done
    }

    pub fn try_send<T>(& mut self, this: & mut Tx<T>, msg: T) -> Result<(), T> {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[monitor::CALL_SINGLE_WRITE]=st.calls[monitor::CALL_SINGLE_WRITE].saturating_add(1);
        }

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

        self.start_hot_profile(monitor::CALL_WAIT);

        let response = this.wait_vacant_units(count).await;

        self.rollup_hot_profile();


        response
    }

    pub async fn send_async<T>(& mut self, this: & mut Tx<T>, a: T) -> Result<(), T> {

       self.start_hot_profile(monitor::CALL_SINGLE_WRITE);

       let result = this.send_async(a).await;

        self.rollup_hot_profile();

       match result  {
           Ok(_) => {
               this.local_index = monitor::process_event(&mut self.telemetry_send_tx
                                                         , this.local_index, this.id,
                    |telemetry, index| telemetry.count[index] = telemetry.count[index].saturating_add(1));
               Ok(())
           },
           Err(sensitive) => {
               error!("Unexpected error send_async telemetry: {} type: {}", self.name, type_name::<T>());
               Err(sensitive)
           }
       }
    }

    fn rollup_hot_profile(&mut self) {
        if let Some(ref mut st) = self.telemetry_state {
            if let Some(d) = st.hot_profile.take() {
                st.await_ns_unit += Instant::elapsed(&d).as_nanos() as u64;

                assert!(st.instant_start.le(&d), "unit_start: {:?} call_start: {:?}", st.instant_start, d);

                //let total_ns = st.instant_start.elapsed().as_nanos() as u64;
                //assert!(total_ns >= st.await_ns_unit, "should be: {} >= {}", total_ns, st.await_ns_unit);
            }
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StdDev(f32);

impl StdDev {
    // Private constructor to directly set the value inside the struct.
    // This is private to ensure that all public constructors go through validation.
    fn new(value: f32) -> Option<Self> {
        if value > 0.0 && value < 10.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub fn one() -> Self {
        Self(1.0)
    }

    pub fn one_and_a_half() -> Self {
        Self(1.5)
    }

    pub fn two() -> Self {
        Self(2.0)
    }

    pub fn two_and_a_half() -> Self {
        Self(2.5)
    }

    pub fn three() -> Self {
        Self(3.0)
    }

    pub fn four() -> Self {
        Self(4.0)
    }

    // Allows custom values within the valid range.
    pub fn custom(value: f32) -> Option<Self> {
        Self::new(value)
    }

    // Getter to access the inner f32 value.
    pub fn value(&self) -> f32 {
        self.0
    }
}

////////////////

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Percentile(f64);

impl Percentile {
    // Private constructor to directly set the value inside the struct.
    // Ensures that all public constructors go through validation.
    fn new(value: f64) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    // Convenience methods for common percentiles
    pub fn p25() -> Self {
        Self(25.0)
    }

    pub fn p50() -> Self {Self(50.0) }

    pub fn p75() -> Self {
        Self(75.0)
    }

    pub fn p90() -> Self {
        Self(90.0)
    }

    pub fn p80() -> Self {
        Self(80.0)
    }

    pub fn p96() -> Self {
        Self(96.0)
    }

    pub fn p99() -> Self {
        Self(99.0)
    }

    // Allows custom values within the valid range.
    pub fn custom(value: f64) -> Option<Self> {
        Self::new(value)
    }

    // Getter to access the inner f32 value.
    pub fn percentile(&self) -> f64 {
        self.0
    }
}

////////////////////

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    // Internal representation as a rational number of the rate per second
    // Numerator: units, Denominator: time in seconds
    numerator: u64,
    denominator: u64,
}

impl Rate {
    // Milliseconds are represented as fractions of a second
    pub fn per_millis(units: u64) -> Self {
        Self {
            numerator: units,
            denominator: 1000,
        }
    }

    pub fn per_seconds(units: u64) -> Self {
        Self {
            numerator: units,
            denominator: 1,
        }
    }

    pub fn per_minutes(units: u64) -> Self {
        Self {
            numerator: units,
            denominator:  60, // 60 seconds
        }
    }

    pub fn per_hours(units: u64) -> Self {
        Self {
            numerator: units,
            denominator:  60 * 60, // 3600 seconds
        }
    }

    pub fn per_days(units: u64) -> Self {
        Self {
            numerator: units,
            denominator: 24 * 60 * 60, // 86400 seconds
        }
    }

    /// Returns the rate as a rational number (numerator, denominator) to represent the rate per second.
    /// This method ensures the rate can be used without performing division, preserving precision.
    pub(crate) fn as_rational_per_second(&self) -> (u64, u64) {
        (self.numerator, self.denominator)
    }
}

///////////////////////////////


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

