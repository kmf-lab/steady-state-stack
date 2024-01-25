use std::any::type_name;
use std::ops::{DerefMut, Sub};
use bastion::context::BastionContext;
use std::time::{Duration, Instant};
use log::error;
use bastion::run;
use futures_timer::Delay;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use futures::lock::Mutex;
use num_traits::Zero;

use crate::steady::config;
use crate::steady::channel::{ChannelBound, ChannelBuilder, SteadyRx};
use crate::steady::channel::SteadyTx;
use crate::steady::config::MAX_TELEMETRY_ERROR_RATE_SECONDS;
use crate::steady::graph::{GraphRuntimeState, SteadyGraph};
use crate::steady::telemetry::metrics_collector::CollectorDetail;

pub struct LocalMonitor<const RX_LEN: usize, const TX_LEN: usize> {
    pub(crate) id: usize, //unique identifier for this child group
    pub(crate) name: & 'static str,
    pub(crate) ctx: Option<BastionContext>,
    pub(crate) telemetry_send_tx: Option<SteadyTelemetrySend<TX_LEN>>,
    pub(crate) telemetry_send_rx: Option<SteadyTelemetrySend<RX_LEN>>,
    pub(crate) last_instant:      Instant,
    pub(crate) runtime_state:     Arc<Mutex<GraphRuntimeState>>,

}

///////////////////
impl <const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {

    pub(crate) fn ctx(&self) -> Option<&BastionContext> {
        if let Some(ctx) = &self.ctx {
            Some(ctx)
        } else {
            None
        }
    }

    pub async fn relay_stats_all(&mut self) {
        //only relay if we are withing the bounds of the telemetry channel limits.
        if self.last_instant.elapsed().as_micros()>= config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u128 {


            let is_in_bastion:bool = {self.ctx().is_some()};

            if let Some(ref mut send_tx) = self.telemetry_send_tx {
                // switch to a new vector and send the old one
                // we always clear the count so we can confirm this in testing

                if send_tx.count.iter().any(|x| !x.is_zero()) {
                    //we only send the result if we have a context, ie a graph we are monitoring
                    if is_in_bastion {

                        let mut lock_guard = send_tx.tx.lock().await;
                        let tx = lock_guard.deref_mut();
                        match tx.try_send(send_tx.count) {
                            Ok(_)  => {
                                send_tx.count.fill(0);
                                if usize::MAX != tx.local_idx { //only record those turned on (init)
                                    send_tx.count[tx.local_idx] = 1;
                                }
                            },
                            Err(a) => {
                                let now = Instant::now();
                                let dif = now.duration_since(send_tx.last_telemetry_error);
                                if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                    //Check metrics_consumer for slowness
                                    error!("relay all tx, full telemetry channel detected upon tx from telemetry: {} value:{:?} full:{} "
                                             , self.name, a, tx.is_full());
                                    //store time to do this again later.
                                    send_tx.last_telemetry_error = now;
                                }
                            }
                        }

                    } else {
                        send_tx.count.fill(0);
                    }
                }
            }

            if let Some(ref mut send_rx) = self.telemetry_send_rx {
                if send_rx.count.iter().any(|x| !x.is_zero()) {
                        //we only send the result if we have a context, ie a graph we are monitoring
                        if is_in_bastion {

                            let mut lock_guard = send_rx.tx.lock().await;
                            let rx = lock_guard.deref_mut();
                            match rx.try_send(send_rx.count) {
                                Ok(_)  => {
                                    send_rx.count.fill(0);
                                    if usize::MAX != rx.local_idx { //only record those turned on (init)
                                        send_rx.count[rx.local_idx] = 1;
                                    }
                                },
                                Err(a) => {
                                    let now = Instant::now();
                                    let dif = now.duration_since(send_rx.last_telemetry_error);
                                    if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                        //Check metrics_consumer for slowness
                                        error!("relay all rx, full telemetry channel detected upon rx from telemetry: {} value:{:?} full:{} "
                                             , self.name, a, rx.is_full());
                                        //store time to do this again later.
                                        send_rx.last_telemetry_error = now;
                                    }
                                }
                            }


                        } else {
                            send_rx.count.fill(0);
                        }
                }
            }
           //note: this is the only place we set this, right after we do it
           self.last_instant = Instant::now();
        }

    }


    pub async fn relay_stats_periodic(self: &mut Self, duration_rate: Duration) {
        assert!(duration_rate.ge(&Duration::from_micros(config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u64)));
        Delay::new(duration_rate.saturating_sub(self.last_instant.elapsed())).await;
        self.relay_stats_all().await;
    }


    pub async fn relay_stats_batch(self: &mut Self) {
        //only relay if one of the channels has reached or passed the capacity
        let doit =
                if let Some(send) = &self.telemetry_send_tx {
                    if send.count.iter().zip(send.limits.iter()).any(|(c,l)|c>=l) {
                        true
                    } else {
                        if let Some(send) = &self.telemetry_send_rx {
                            send.count.iter().zip(send.limits.iter()).any(|(c,l)| c>=l)
                        } else {
                            false
                        }
                    }
                } else {
                    if let Some(send) = &self.telemetry_send_rx {
                        send.count.iter().zip(send.limits.iter()).any(|(c,l)| c>=l)
                    } else {
                        false
                    }
                };
        if doit {
            self.relay_stats_all().await;
        }
    }

    pub fn relay_stats_tx_set_custom_batch_limit<T>(self: &mut Self, tx: &SteadyTx<T>, threshold: usize) {
        if let Some(ref mut send) = self.telemetry_send_tx {
            if usize::MAX != tx.local_idx {
                send.limits[tx.local_idx] = threshold;
            }
        }
    }

    pub fn relay_stats_rx_set_custom_batch_limit<T>(&mut self, rx: &SteadyRx<T>, threshold: usize) {
        if let Some(ref mut send) = self.telemetry_send_rx {
            if usize::MAX != rx.local_idx {
                send.limits[rx.local_idx] = threshold;
            }
        }
    }

    pub fn take_slice<T>(& mut self, this: & mut SteadyRx<T>, slice: &mut [T]) -> usize
        where T: Copy {
        let done = this.take_slice(slice);
        if let Some(ref mut telemetry) = self.telemetry_send_rx {
            if usize::MAX != this.local_idx { //only record those turned on (init)
                let count = telemetry.count[this.local_idx];
                telemetry.count[this.local_idx] = count.saturating_add(done);
            }
        }
        done
    }

    pub fn try_take<T>(& mut self, this: & mut SteadyRx<T>) -> Option<T> {
        match this.try_take() {
            Some(msg) => {
                if let Some(ref mut telemetry) = self.telemetry_send_rx {
                    if usize::MAX != this.local_idx { //only record those turned on (init)
                        let count = telemetry.count[this.local_idx];
                        telemetry.count[this.local_idx] = count.saturating_add(1);
                    }
                }
                Some(msg)
            },
            None => {None}
        }
    }
    pub fn try_peek<'a,T>(&'a mut self, this: &'a mut SteadyRx<T>) -> Option<&T> {
        this.try_peek()  //nothing to record since nothing moved. TODO: revisit this
    }
    pub fn is_empty<T>(& mut self, this: & mut SteadyRx<T>) -> bool {
        this.is_empty()
    }

    pub fn avail_units<T>(& mut self, this: & mut SteadyRx<T>) -> usize {
        this.avail_units()
    }

    pub async fn wait_avail_units<T>(& mut self, this: & mut SteadyRx<T>, count:usize) {
        this.wait_avail_units(count).await
    }


    pub async fn peek_async<'a,T>(&'a mut self, this: &'a mut SteadyRx<T>) -> Option<&T> {
        //nothing to record since nothing moved. TODO: revisit this
        this.peek_async().await

    }
    pub async fn take_async<T>(& mut self, this: & mut SteadyRx<T>) -> Result<T, String> {
        match this.take_async().await {
            Ok(result) => {
                if let Some(ref mut telemetry) = self.telemetry_send_rx {
                        if usize::MAX != this.local_idx { //only record those turned on (init)
                            let count = telemetry.count[this.local_idx];
                            telemetry.count[this.local_idx] = count.saturating_add(1);
                        }
                }
                Ok(result)
            },
            Err(error_msg) => {
                error!("Unexpected error take_async: {} {}", error_msg, self.name);
                Err(error_msg)
            }
        }
    }

    pub fn send_slice_until_full<T>(&mut self, this: & mut SteadyTx<T>, slice: &[T]) -> usize
        where T: Copy {
        let done = this.send_slice_until_full(slice);
        if let Some(ref mut telemetry) = self.telemetry_send_tx {
            if usize::MAX != this.local_idx { //only record those turned on (init)
                let count = telemetry.count[this.local_idx];
                telemetry.count[this.local_idx] = count.saturating_add(done);
            }
        }
        done
    }

    pub fn send_iter_until_full<T,I: Iterator<Item = T>>(&mut self, this: & mut SteadyTx<T>, iter: I) -> usize {
        let done = this.send_iter_until_full(iter);
        if let Some(ref mut telemetry) = self.telemetry_send_tx {
            if usize::MAX != this.local_idx { //only record those turned on (init)
                let count = telemetry.count[this.local_idx];
                telemetry.count[this.local_idx] = count.saturating_add(done);
            }
        }
        done
    }


    pub fn try_send<T>(& mut self, this: & mut SteadyTx<T>, msg: T) -> Result<(), T> {
        match this.try_send(msg) {
            Ok(_) => {
                if let Some(ref mut telemetry) = self.telemetry_send_tx {
                    if usize::MAX != this.local_idx { //only record those turned on (init)
                        let count = telemetry.count[this.local_idx];
                        telemetry.count[this.local_idx] = count.saturating_add(1);
                    }
                }
                Ok(())
            },
            Err(sensitive) => {
                error!("Unexpected error try_send  telemetry: {} type: {}"
                    , self.name, type_name::<T>());
                Err(sensitive)
            }
        }
    }
    pub fn is_full<T>(& mut self, this: & mut SteadyTx<T>) -> bool {
        this.is_full()
    }

    pub fn vacant_units<T>(& mut self, this: & mut SteadyTx<T>) -> usize {
        this.vacant_units()
    }
    pub async fn wait_vacant_units<T>(& mut self, this: & mut SteadyTx<T>, count:usize) {
        this.wait_vacant_units(count).await
    }

    pub async fn send_async<T>(& mut self, this: & mut SteadyTx<T>, a: T) -> Result<(), T> {
       match this.send_async(a).await {
           Ok(_) => {
               if let Some(ref mut telemetry) = self.telemetry_send_tx {
                   if usize::MAX != this.local_idx { //only record those turned on (init)
                       let count = telemetry.count[this.local_idx];
                       telemetry.count[this.local_idx] = count.saturating_add(1);
                   }
               }
               Ok(())
           },
           Err(sensitive) => {
               error!("Unexpected error send_async  telemetry: {} type: {}"
                   , self.name, type_name::<T>());
               Err(sensitive)
           }
       }

    }


}

pub struct SteadyMonitor {
    pub(crate) id: usize, //unique identifier for this child group
    pub(crate) name: & 'static str,
    pub(crate) ctx: Option<BastionContext>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<Mutex<GraphRuntimeState>>,
}

impl SteadyMonitor {

    pub fn init_stats<const RX_LEN: usize, const TX_LEN: usize>(self
                           , rx_tag: &mut [& mut dyn RxDef; RX_LEN]
                           , tx_tag: &mut [& mut dyn TxDef; TX_LEN]
    ) -> LocalMonitor<RX_LEN,TX_LEN> {

        //only build telemetry channels if this feature is enabled
        let (telemetry_send_rx, telemetry_send_tx) = if config::TELEMETRY_HISTORY || config::TELEMETRY_SERVER {
             self.build_telemetry_channels(rx_tag, tx_tag)
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
        }
    }

    fn build_telemetry_channels<const RX_LEN: usize, const TX_LEN: usize>(& self, rx_tag: &mut [&mut dyn RxDef; RX_LEN], tx_tag: &mut [&mut dyn TxDef; TX_LEN]) -> (Option<SteadyTelemetrySend<{ RX_LEN }>>, Option<SteadyTelemetrySend<{ TX_LEN }>>) {
        let mut rx_batch_limit = [0; RX_LEN];
        let mut rx_meta_data = Vec::new();
        rx_tag.iter_mut()
            .enumerate()
            .for_each(|(c, rx)| {
                rx.set_local_id(c);
                rx_batch_limit[c] = rx.batch_limit();
                rx_meta_data.push(rx.meta_data());
            });

        let mut tx_batch_limit = [0; TX_LEN];
        let mut tx_meta_data = Vec::new();
        tx_tag.iter_mut()
            .enumerate()
            .for_each(|(c, tx)| {
                tx.set_local_id(c, self.name);
                tx_batch_limit[c] = tx.batch_limit();
                tx_meta_data.push(tx.meta_data());
            });

        //NOTE: if this child telemetry is monitored so we will create the appropriate channels

        let rx_tuple: (Option<SteadyTelemetrySend<RX_LEN>>, Option<SteadyTelemetryTake<RX_LEN>>)
            = if RX_LEN.is_zero() {
            (None, None)
        } else {
            let (telemetry_send_rx, mut telemetry_take_rx) =
                ChannelBuilder::new(self.channel_count.clone(), config::REAL_CHANNEL_LENGTH_TO_COLLECTOR)
                    .with_labels(&["steady-telemetry"], false)
                    .build();

            (Some(SteadyTelemetrySend::new(telemetry_send_rx, [0; RX_LEN], rx_batch_limit), )
             , Some(SteadyTelemetryTake { rx: telemetry_take_rx, details: rx_meta_data }))
        };

        let tx_tuple: (Option<SteadyTelemetrySend<TX_LEN>>, Option<SteadyTelemetryTake<TX_LEN>>)
            = if TX_LEN.is_zero() {
            (None, None)
        } else {
            let (telemetry_send_tx, telemetry_take_tx) =
                ChannelBuilder::new(self.channel_count.clone(), config::REAL_CHANNEL_LENGTH_TO_COLLECTOR)
                    .with_labels(&["steady-telemetry"], false)
                    .build();

            (Some(SteadyTelemetrySend::new(telemetry_send_tx, [0; TX_LEN], tx_batch_limit), )
             , Some(SteadyTelemetryTake { rx: telemetry_take_tx, details: tx_meta_data }))
        };


        let details = CollectorDetail {
            name: self.name
            ,
            monitor_id: self.id
            ,
            telemetry_take: Box::new(SteadyTelemetryRx {
                send: tx_tuple.1,
                take: rx_tuple.1,
            })
        };

        //need to hand off to the collector
        run!(async{
                let mut shared_vec_guard = self.all_telemetry_rx.lock().await;
                let shared_vec = shared_vec_guard.deref_mut();
                let index:Option<usize> = shared_vec.iter()
                                        .enumerate()
                                        .find(|(_,x)|x.monitor_id == self.id && x.name == self.name)
                                        .map(|(idx,_)|idx);
                if let Some(idx) = index {
                    shared_vec[idx] = details; //replace old telemetry channels
                } else {
                    shared_vec.push(details); //add new telemetry channels
                }
            });

        let telemetry_send_rx = rx_tuple.0;
        let telemetry_send_tx = tx_tuple.0;
        (telemetry_send_rx, telemetry_send_tx)
    }

    //TODO: add methods later to instrument non await blocks for cpu usage chart

}

pub struct SteadyTelemetryRx<const RXL: usize, const TXL: usize> {
    send: Option<SteadyTelemetryTake<TXL>>,
    take: Option<SteadyTelemetryTake<RXL>>,
}

impl <const RXL: usize, const TXL: usize> RxTel for SteadyTelemetryRx<RXL,TXL> {

    #[inline]
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
        if let Some(send) = &self.send {
            send.details.to_vec()
        } else {
            vec![]
        }
    }
    #[inline]
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
        if let Some(take) = &self.take {
            take.details.to_vec()
        } else {
            vec![]
        }
    }


    #[inline]
    fn consume_into(&self, take_target: &mut Vec<i128>, send_target: &mut Vec<i128>) {

         //this method can not be async since we need vtable and dyn
        //TODO: revisit later we may be able to return a closure instead


            if let Some(ref take) = &self.take {
                let mut buffer = [[0usize;RXL];config::LOCKED_CHANNEL_LENGTH_TO_COLLECTOR];

                let count = {
                    let mut rx_guard = run!(take.rx.lock());
                    let rx = rx_guard.deref_mut();

                   /* let mut c = 0;
                    while let Some(m) = rx.try_take() {
                        buffer[c] = m;
                        c += 1;
                        if config::LOCKED_CHANNEL_LENGTH_TO_COLLECTOR == c {
                            break;
                        }
                    }
                    c*/
                    rx.take_slice( & mut buffer)
                };
                let populated_slice = &buffer[0..count];

                populated_slice.iter().for_each(|msg| {
                    take.details.iter()
                        .zip(msg.iter())
                        .for_each(|(meta, val)| {
                            if meta.id>=take_target.len() {
                                take_target.resize(1 + meta.id, 0);
                            };
                            //info!("count rx {}+{}",take_target[meta.id],val);
                            take_target[meta.id] += *val as i128;
                        });
                });

            }
            if let Some(ref send) = &self.send {
                let mut buffer = [[0usize;TXL];config::LOCKED_CHANNEL_LENGTH_TO_COLLECTOR];

                let count = {
                    let mut tx_guard = run!(send.rx.lock());
                    let tx = tx_guard.deref_mut();

                    /*let mut c = 0;
                    while let Some(m) = tx.try_take() {
                        buffer[c] = m;
                        c += 1;
                        if config::LOCKED_CHANNEL_LENGTH_TO_COLLECTOR == c {
                            break;
                        }
                    }
                    c*/

                    tx.take_slice( & mut buffer)

                };
                let populated_slice = &buffer[0..count];

                populated_slice.iter().for_each(|msg| {
                    send.details.iter()
                        .zip(msg.iter())
                        .for_each(|(meta, val)| {
                            if meta.id>=send_target.len() {
                                send_target.resize(1 + meta.id, 0);
                            };
                            //info!("count tx {}+{}",take_target[meta.id],val);
                            send_target[meta.id] += *val as i128;

                        });
                });


            }

    }

    fn biggest_tx_id(&self) -> usize {
        if let Some(tx) = &self.send {
            *tx.details.iter().map(|m|&m.id).max().unwrap_or(&0)
        } else {
            0
        }

    }

    fn biggest_rx_id(&self) -> usize {
        if let Some(tx) = &self.take {
            *tx.details.iter().map(|m|&m.id).max().unwrap_or(&0)
        } else {
            0
        }
    }
}

pub struct SteadyTelemetrySend<const LENGTH: usize> {
    pub(crate) tx: Arc<Mutex<SteadyTx<[usize; LENGTH]>>>,
    pub(crate) count: [usize; LENGTH],
    pub(crate) limits: [usize; LENGTH],
    pub(crate) last_telemetry_error: Instant,
}

impl <const LENGTH: usize> SteadyTelemetrySend<LENGTH> {
    pub fn new(tx: Arc<Mutex<SteadyTx<[usize; LENGTH]>>>,
               count: [usize; LENGTH],
               limits: [usize; LENGTH]) -> SteadyTelemetrySend<LENGTH> {
        SteadyTelemetrySend{
             tx,count,limits
            ,last_telemetry_error: Instant::now().sub(Duration::from_secs(1+MAX_TELEMETRY_ERROR_RATE_SECONDS as u64))
        }
    }
}

#[derive(Clone)]
pub(crate) struct ChannelMetaData {
    pub(crate) id: usize,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) display_labels: bool, //TODO: bit mask
    pub(crate) line_expansion: bool,
    pub(crate) show_type: bool,
    pub(crate) window_in_seconds: u64, //for percentiles and ma
    pub(crate) percentiles: Vec<u8>, //each is a row
    pub(crate) std_dev: Vec<f32>, //each is a row
    pub(crate) red: Option<ChannelBound>, //if used base is green
    pub(crate) yellow: Option<ChannelBound>, //if used base is green
}

pub struct SteadyTelemetryTake<const LENGTH: usize> {
    rx: Arc<Mutex<SteadyRx<[usize; LENGTH]>>>,
    details: Vec<Arc<ChannelMetaData>>,
}

pub trait RxDef {
    fn meta_data(&self) -> Arc<ChannelMetaData>;
    fn batch_limit(&self) -> usize;
    fn set_local_id(& mut self, idx: usize);
}

impl <T> RxDef for SteadyRx<T> {

    fn meta_data(&self) -> Arc<ChannelMetaData> {
        self.channel_meta_data.clone()
    }
    fn batch_limit(&self) -> usize {
        self.batch_limit
    }

    fn set_local_id(& mut self, idx: usize) {
        self.local_idx = idx;
    }
}

pub trait TxDef {
    fn meta_data(&self) -> Arc<ChannelMetaData>;
    fn batch_limit(&self) -> usize;
    fn set_local_id(& mut self, idx: usize, actor_name: &'static str);

}

impl <T> TxDef for SteadyTx<T> {

    fn meta_data(&self) -> Arc<ChannelMetaData> {
        self.channel_meta_data.clone()
    }
    fn batch_limit(&self) -> usize {
        self.batch_limit
    }
    fn set_local_id(& mut self, idx: usize, actor_name: &'static str) {
        self.local_idx = idx;
        self.actor_name = Some(actor_name);
    }

}


pub trait RxTel : Send + Sync {


    //returns an iterator of usize channel ids
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    fn consume_into(&self, take_target: &mut Vec<i128>, send_target: &mut Vec<i128>);

        //NOTE: we will do one dyn call per node every 32ms or so to build the image
    //      we only have 1 impl assuming the compiler will inline this if possible
    // TODO: in the future we could rewrite this to return a future that can be pinned and boxed
 //   fn consume_into(& mut self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>);
    fn biggest_tx_id(&self) -> usize;
    fn biggest_rx_id(&self) -> usize;

}

#[cfg(test)]
pub(crate) mod monitor_tests {
    use std::ops::DerefMut;
    use crate::steady::*;
    use async_std::test;
    use lazy_static::lazy_static;
    use std::sync::Once;
    use std::time::Duration;
    use futures_timer::Delay;
    use crate::steady::channel::SteadyRx;
    use crate::steady::channel::SteadyTx;
    use crate::steady::graph::SteadyGraph;
    lazy_static! {
            static ref INIT: Once = Once::new();
    }

    //this is my unit test for relay_stats_tx_custom
    #[test]
    async fn test_relay_stats_tx_rx_custom() {
        crate::steady::util::util_tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_string, rx_string) = graph.channel_builder(8)
            .build();

        let monitor = graph.new_test_monitor("test");
        let mut rx_string_guard = rx_string.lock().await;
        let mut tx_string_guard = tx_string.lock().await;

        let rxd: &mut SteadyRx<String> = rx_string_guard.deref_mut();
        let txd: &mut SteadyTx<String> = tx_string_guard.deref_mut();

        let mut monitor = monitor.init_stats(&mut[rxd], &mut[txd]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string()).await;
            count += 1;
        }

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_idx], threshold);
        }
        monitor.relay_stats_tx_set_custom_batch_limit(txd, threshold);
        monitor.relay_stats_batch().await;

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_idx], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], threshold);
        }
        monitor.relay_stats_rx_set_custom_batch_limit(rxd, threshold);
        Delay::new(Duration::from_micros(config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u64)).await;

        monitor.relay_stats_batch().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], 0);
        }
    }

    #[test]
    async fn test_relay_stats_tx_rx_batch() {
        crate::steady::util::util_tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let monitor = graph.new_test_monitor("test");

        let (tx_string, rx_string) = graph.channel_builder(5).build();

        let mut rx_string_guard = rx_string.lock().await;
        let mut tx_string_guard = tx_string.lock().await;

        let rxd: &mut SteadyRx<String> = rx_string_guard.deref_mut();
        let txd: &mut SteadyTx<String> = tx_string_guard.deref_mut();

        let mut monitor = monitor.init_stats(&mut [rxd], &mut [txd]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string()).await;
            count += 1;
            if let Some(ref mut tx) = monitor.telemetry_send_tx {
                assert_eq!(tx.count[txd.local_idx], count);
            }
            monitor.relay_stats_batch().await;
        }

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_idx], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }
        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], threshold);
        }
        Delay::new(Duration::from_micros(config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u64)).await;

        monitor.relay_stats_batch().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_idx], 0);
        }
    }
}