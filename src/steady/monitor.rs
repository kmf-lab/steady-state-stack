use std::any::type_name;
use std::ops::{DerefMut, Sub};
use bastion::context::BastionContext;
use std::time::{Duration, Instant};
use log::error;
use bastion::run;
use futures_timer::Delay;
use petgraph::matrix_graph::Zero;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use async_std::sync::Mutex;
use crate::steady;
use crate::steady::{config, MAX_TELEMETRY_ERROR_RATE_SECONDS, SteadyRx, SteadyTx};
use crate::steady::telemetry::metrics_collector::{CollectorDetail, RxTel};

pub struct LocalMonitor<const RX_LEN: usize, const TX_LEN: usize> {
    pub(crate) id: usize, //unique identifier for this child group
    pub(crate) name: & 'static str,
    pub(crate) ctx: Option<BastionContext>,
    pub(crate) telemetry_send_tx: Option<SteadyTelemetrySend<TX_LEN>>,
    pub(crate) telemetry_send_rx: Option<SteadyTelemetrySend<RX_LEN>>,
    pub(crate) last_instant:      Instant,
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

        // do not run any faster than the framerate of the telemetry can consume
        //TODO: this is not right needs alignment with the periodic counter and other calls
        //let run_duration: Duration = Instant::now().duration_since(self.monitor.last_instant);
        //if run_duration.as_millis() as usize >= steady_feature::MIN_TELEMETRY_CAPTURE_RATE_MS
        {
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
                                    error!("full telemetry channel detected upon tx from telemetry: {} value:{:?} "
                                       , self.name, a);
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
                                         error!("full telemetry channel detected upon rx from telemetry: {} value:{:?} full:{} "
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
        }
    }


    pub async fn relay_stats_periodic(self: &mut Self, duration_rate: Duration) {

        assert_eq!(true, duration_rate.ge(&Duration::from_millis(config::MIN_TELEMETRY_CAPTURE_RATE_MS as u64)));

        let run_duration: Duration = {
                Instant::now().duration_since(self.last_instant).clone()
            };

        run!(
            async {
        Delay::new(duration_rate.saturating_sub(run_duration)).await;

            }
        );

        {
            self.last_instant = Instant::now();
            self.relay_stats_all().await;
        }


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

    pub fn relay_stats_rx_set_custom_batch_limit<T>(self: &mut Self, rx: &SteadyRx<T>, threshold: usize) {
        if let Some(ref mut send) = self.telemetry_send_rx {
            if usize::MAX != rx.local_idx {
                send.limits[rx.local_idx] = threshold;
            }
        }
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

    pub fn try_send<T>(self: & mut Self, this: & mut SteadyTx<T>, msg: T) -> Result<(), T> {
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
    pub fn is_full<T>(self: & mut Self,this: & mut SteadyTx<T>) -> bool {
        this.is_full()
    }

    pub fn vacant_units<T>(self: & mut Self,this: & mut SteadyTx<T>) -> usize {
        this.vacant_units()
    }
    pub async fn wait_vacant_units<T>(self: & mut Self,this: & mut SteadyTx<T>, count:usize) {
        this.wait_vacant_units(count).await
    }

    pub async fn send_async<T>(self: & mut Self, this: & mut SteadyTx<T>, a: T) -> Result<(), T> {
       match this.send_async(a).await {
           Ok(_) => {
               if let Some(ref mut telemetry) = self.telemetry_send_tx {
                   if usize::MAX != this.local_idx { //only record those turned on (init)
                       let count = telemetry.count[this.local_idx];
                       telemetry.count[this.local_idx] = count.saturating_add(1);
                   }
               }
               return Ok(())
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
    pub(crate) all_telemetry_rx: Arc<Mutex<Vec<CollectorDetail>>>
}

impl SteadyMonitor {

    pub fn init_stats<const RX_LEN: usize, const TX_LEN: usize>(self
                                   , rx_tag: &mut [& mut dyn RxDef; RX_LEN]
                                   , tx_tag: &mut [& mut dyn TxDef; TX_LEN]
    ) -> LocalMonitor<RX_LEN,TX_LEN> {

        let mut rx_batch_limit = [0; RX_LEN];
        let mut map_rx = Vec::new();
        rx_tag.iter_mut()
            .enumerate()
            .for_each(|(c, rx)| {
                rx_batch_limit[c] = rx.batch_limit();
                map_rx.push(rx.meta_data());
                rx.set_local_id(c);
            });


        let mut tx_batch_limit = [0; TX_LEN];
        let mut map_tx = Vec::new();
        tx_tag.iter_mut()
            .enumerate()
            .for_each(|(c, tx)| {
                tx.set_local_id(c, self.name);
                tx_batch_limit[c] = tx.batch_limit();
                map_tx.push(tx.meta_data());
            });


        //NOTE: if this child telemetry is monitored so we will create the appropriate channels

        let rx_tuple: (Option<SteadyTelemetrySend<RX_LEN>>, Option<SteadyTelemetryTake<RX_LEN>>)
            = if RX_LEN.is_zero() {
                (None,None)
            } else {
                let (telemetry_send_rx, mut telemetry_take_rx)
                    = steady::build_channel(self.channel_count.fetch_add(1, Ordering::SeqCst)
                                            , config::CHANNEL_LENGTH_TO_COLLECTOR
                                            , &["steady-telemetry"]
                                    );
                ( Some(SteadyTelemetrySend::new(telemetry_send_rx, [0; RX_LEN],  rx_batch_limit),)
                 ,Some(SteadyTelemetryTake{rx: telemetry_take_rx, map: map_rx })  )
            };

        let tx_tuple: (Option<SteadyTelemetrySend<TX_LEN>>, Option<SteadyTelemetryTake<TX_LEN>>)
            = if TX_LEN.is_zero() {
                 (None,None)
        } else {
            let (telemetry_send_tx, telemetry_take_tx)
                = steady::build_channel(self.channel_count.fetch_add(1, Ordering::SeqCst)
                                        , config::CHANNEL_LENGTH_TO_COLLECTOR
                                        , &["steady-telemetry"]
                                );
            ( Some(SteadyTelemetrySend::new(telemetry_send_tx, [0; TX_LEN], tx_batch_limit),)
              ,Some(SteadyTelemetryTake{rx: telemetry_take_tx, map: map_tx })  )
        };


        let details = CollectorDetail{
              name: self.name
            , monitor_id: self.id
            , telemetry_take: Box::new(SteadyTelemetryRx {
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

         // this is my locked version for this specific thread
        LocalMonitor::<RX_LEN, TX_LEN> {
            telemetry_send_rx,  telemetry_send_tx,
            last_instant: Instant::now(),
            id: self.id,
            name: self.name,
            ctx: self.ctx,
        }
    }

    //TODO: add methods later to instrument non await blocks for cpu usage chart

}

pub struct SteadyTelemetryRx<const RXL: usize, const TXL: usize> {
    send: Option<SteadyTelemetryTake<TXL>>,
    take: Option<SteadyTelemetryTake<RXL>>,
}

impl <const RXL: usize, const TXL: usize> RxTel for SteadyTelemetryRx<RXL,TXL> {

    #[inline]
    fn tx_channel_id_vec(&self) -> Vec<(usize, Vec<&'static str>)> {
        if let Some(send) = &self.send {
            send.map.to_vec()
        } else {
            vec![]
        }
    }
    #[inline]
    fn rx_channel_id_vec(&self) -> Vec<(usize, Vec<&'static str>)> {
        if let Some(take) = &self.take {
            take.map.to_vec()
        } else {
            vec![]
        }
    }


    #[inline]
    fn consume_into(&self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>) {

         //this method can not be async since we need vtable and dyn
        //TODO: revisit later we may be able to return a closure instead

            if let Some(ref take) = &self.take {
                let mut rx_guard = run!(take.rx.lock());
                let rx = rx_guard.deref_mut();
                loop {
                    if let Some(msg) = rx.try_take() {
                        take.map.iter()
                            .zip(msg.iter())
                            .for_each(|(meta, val)| {
                                if meta.0>=take_target.len() {
                                    take_target.resize(1 + meta.0, 0);
                                }
                                take_target[meta.0] += *val as u128
                            });
                    } else {
                        break;
                    }
                }
            }
            if let Some(ref send) = &self.send {
                let mut tx_guard = run!(send.rx.lock());
                let tx = tx_guard.deref_mut();
                loop {
                    if let Some(msg) = tx.try_take() {
                        send.map.iter()
                            .zip(msg.iter())
                            .for_each(|(meta, val)| {
                                if meta.0>=take_target.len() {
                                    take_target.resize(1 + meta.0, 0);
                                }
                                take_target[meta.0] += *val as u128
                            });
                    } else {
                        break;
                    }
                }
            }

    }

    fn biggest_tx_id(&self) -> usize {
        if let Some(tx) = &self.send {
            tx.map.iter().map(|(i,l)|i).max().unwrap_or(&0).clone()
        } else {
            0
        }

    }

    fn biggest_rx_id(&self) -> usize {
        if let Some(tx) = &self.take {
            tx.map.iter().map(|(i,l)|i).max().unwrap_or(&0).clone()
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

pub struct SteadyTelemetryTake<const LENGTH: usize> {
    rx: Arc<Mutex<SteadyRx<[usize; LENGTH]>>>,
    map: Vec<(usize, Vec<&'static str>)>,
}

pub trait RxDef {
    fn meta_data(&self) -> (usize, Vec<&'static str>);
    fn batch_limit(&self) -> usize;
    fn set_local_id(& mut self, idx: usize);
}

impl <T> RxDef for SteadyRx<T> {

    fn meta_data(&self) -> (usize, Vec<&'static str>) {
        (self.id, Vec::from(self.label))
    }
    fn batch_limit(&self) -> usize {
        self.batch_limit
    }

    fn set_local_id(& mut self, idx: usize) {
        self.local_idx = idx;
    }
}

pub trait TxDef {
    fn meta_data(&self) -> (usize, Vec<&'static str>);
    fn batch_limit(&self) -> usize;
    fn set_local_id(& mut self, idx: usize, actor_name: &'static str);

}

impl <T> TxDef for SteadyTx<T> {

    fn meta_data(&self) -> (usize, Vec<&'static str>) {
        (self.id, Vec::from(self.label))
    }
    fn batch_limit(&self) -> usize {
        self.batch_limit
    }
    fn set_local_id(& mut self, idx: usize, actor_name: &'static str) {
        self.local_idx = idx;
        self.actor_name = Some(actor_name);
    }

}
