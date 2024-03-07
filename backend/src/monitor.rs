use std::any::type_name;

#[cfg(test)]
use std::collections::HashMap;

use std::ops::*;
use std::time::{Duration, Instant};

use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU32, Ordering};
use futures::lock::Mutex;
#[allow(unused_imports)]
use log::*; //allowed for all modules
use num_traits::Zero;
use futures::future::{pending, select_all};
use std::task::Context;
use futures_timer::Delay;
use std::future::Future;
use futures::FutureExt;
use crate::{AlertColor, config, MONITOR_NOT, MONITOR_UNKNOWN, Rx, RxDef, StdDev, SteadyRx, SteadyRxBundle, SteadyTx, SteadyTxBundle, telemetry, Trigger, Tx};
use crate::actor_builder::{MCPU, Percentile, Work};
use crate::channel_builder::{Filled, Rate};
use crate::graph_liveliness::{ActorIdentity, GraphLiveliness};
use crate::graph_testing::EdgeSimulator;


pub struct SteadyTelemetryRx<const RXL: usize, const TXL: usize> {
    pub(crate) send: Option<SteadyTelemetryTake<TXL>>,
    pub(crate) take: Option<SteadyTelemetryTake<RXL>>,
    pub(crate) actor: Option<SteadyRx<ActorStatus>>,
    pub(crate) actor_metadata: Arc<ActorMetaData>,
}
pub struct SteadyTelemetryTake<const LENGTH: usize> {
    pub(crate) rx: Arc<Mutex<Rx<[usize; LENGTH]>>>,
    pub(crate) details: Vec<Arc<ChannelMetaData>>,
}

#[derive(Clone,Copy,Debug,Default,Eq,PartialEq)]
pub struct ActorStatus {
    pub(crate) total_count_restarts: u32, //always max so just pick the latest
    pub(crate) bool_stop:            bool, //always max so just pick the latest

    pub(crate) await_total_ns:       u64,  //sum records together
    pub(crate) unit_total_ns:        u64,  //sum records together
    pub(crate) redundancy:           u16,

    pub(crate) calls: [u16; 6],
}

pub(crate) const CALL_SINGLE_READ: usize=0;
pub(crate) const CALL_BATCH_READ: usize=1;
pub(crate) const CALL_SINGLE_WRITE: usize=2;
pub(crate) const CALL_BATCH_WRITE: usize=3;
pub(crate) const CALL_OTHER: usize=4;
pub(crate) const CALL_WAIT: usize=5;

pub struct SteadyTelemetryActorSend {
    pub(crate) tx: SteadyTx<ActorStatus>,
    pub(crate) last_telemetry_error: Instant,

    pub(crate) await_ns_unit: u64,
    pub(crate) instant_start: Instant,
    pub(crate) hot_profile:   Option<Instant>,

    pub(crate) redundancy: u16,


    pub(crate) calls: [u16; 6],

    pub(crate) count_restarts: Arc<AtomicU32>,
    pub(crate) bool_stop: bool,
}

impl SteadyTelemetryActorSend {

    pub(crate) fn status_reset(&mut self) {


        //ok on shutdown. confirm we are shutting down.
        //assert!(self.hot_profile.is_none(),"internal error");

        self.await_ns_unit = 0;
        self.instant_start = Instant::now();
        self.calls.fill(0);
    }

    pub(crate) fn status_message(&self) -> ActorStatus {

            let total_ns = self.instant_start.elapsed().as_nanos() as u64;

            //ok on shutdown.
            //assert!(self.hot_profile.is_none(),"internal error");

            assert!(total_ns>=self.await_ns_unit,"should be: {} >= {}",total_ns,self.await_ns_unit);
            ActorStatus {
                total_count_restarts: self.count_restarts.load(Ordering::Relaxed),
                bool_stop: self.bool_stop,
                await_total_ns: self.await_ns_unit,
                unit_total_ns: total_ns,
                redundancy: self.redundancy,
                calls: self.calls,
            }

    }

}

pub struct SteadyTelemetrySend<const LENGTH: usize> {
    pub(crate) tx: SteadyTx<[usize; LENGTH]>,
    pub(crate) count: [usize; LENGTH],
    pub(crate) last_telemetry_error: Instant,
    pub(crate) inverse_local_index: [usize; LENGTH],
}


impl <const RXL: usize, const TXL: usize> RxTel for SteadyTelemetryRx<RXL,TXL> {

    fn actor_metadata(&self) -> Arc<ActorMetaData> {
            self.actor_metadata.clone()
    }

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

    fn actor_rx(&self) -> Option<Box<dyn RxDef>> {
        if let Some(ref act) = &self.actor {
            Some(Box::new(act.clone()))
        } else {
            None
        }
    }

    fn consume_actor(&self) -> Option<ActorStatus> {
        if let Some(ref act) = &self.actor {

            let mut buffer = [ActorStatus::default();config::CONSUMED_MESSAGES_BY_COLLECTOR +1];
            let count = {
                let mut guard = bastion::run!(act.lock());
                let act = guard.deref_mut();
                act.take_slice( & mut buffer)
            };

            let mut await_total_ns:       u64 = 0;
            let mut unit_total_ns:        u64 = 0;

            let mut calls = [0u16;6];
            for status in buffer.iter().take(count) {
                assert!(status.unit_total_ns>=status.await_total_ns,"{} {}",status.unit_total_ns,status.await_total_ns);

                await_total_ns += status.await_total_ns;
                unit_total_ns += status.unit_total_ns;

                for (i, call) in status.calls.iter().enumerate() {
                    calls[i] = calls[i].saturating_add(*call);
                }
            }
            if count>0 {
                Some(ActorStatus {
                    total_count_restarts:  buffer[count - 1].total_count_restarts,
                    bool_stop: buffer[count - 1].bool_stop,
                    redundancy: buffer[count - 1].redundancy,
                    await_total_ns,
                    unit_total_ns,
                    calls,
                })
            } else {
                None
            }
        } else {
            None
        }
    }


    #[inline]
    fn consume_take_into(&self, take_send_source: &mut Vec<(i128,i128)>
                         , future_take: &mut Vec<i128>
                         , future_send: &mut Vec<i128>
                    ) -> bool {
        if let Some(ref take) = &self.take {
            let mut buffer = [[0usize;RXL];config::CONSUMED_MESSAGES_BY_COLLECTOR +1];

            let count = {
                let mut rx_guard = bastion::run!(take.rx.lock());
                let rx = rx_guard.deref_mut();
                rx.take_slice( & mut buffer)
            };
            let populated_slice = &buffer[0..count];


            //this logic is key because we must find the break point between frames.
            //we use send as a boundary to know that these takes on the same channel
            //either belong to this frame or the next frame.  Data from the last frame
            //is then used to start our new target data.

            //this is essential to ensure send>=take for all frames before we generate labels
            //this also ensures the data is highly compressible

            //further the count on the channel is send-take so we must also be sure that
            //send cannot be bigger than take+capacity. This will be adjusted at the end


            //first pull the data we can from last frame into here
            take.details.iter().for_each(|meta| {
                //note we may be spanning the boundary of a failed actor
                //if that is the case we may not be able to pick up all the data
                //and must leave some for future frames
                let max_takeable = take_send_source[meta.id].1-take_send_source[meta.id].0;
                assert!(max_takeable.ge(&0),"internal error");
                let value_taken = max_takeable.min(future_take[meta.id]);
                take_send_source[meta.id].0 += value_taken;
                future_take[meta.id] -= value_taken;
            });

            //pull in new data for this frame
            populated_slice.iter().for_each(|msg| {
                take.details.iter()
                    .zip(msg.iter())
                    .for_each(|(meta, val)| {
                        let limit = take_send_source[meta.id].1;
                        let val = *val as i128;
                        //we can go up to the limit but no more
                        //once we hit the limit we start putting the data into the future
                        if i128::is_zero(&future_take[meta.id])
                            && val+take_send_source[meta.id].0 <= limit {
                                take_send_source[meta.id].0 += val;
                        } else {
                                future_take[meta.id] += val;
                        }
                    });
            });

            //check all values and ensure we do not have more than capacity between send and take
            take.details.iter().for_each(|meta| {
                let dif = take_send_source[meta.id].1-take_send_source[meta.id].0;
                if dif > (meta.capacity as i128) {
                    //trace!("too many sends this frame, pushing some off to next frame");
                    let extra = dif - (meta.capacity as i128);
                    future_send[meta.id] += extra;
                    take_send_source[meta.id].1 -= extra; //adjust the send
                }
            });

            count>0
        } else {
            false
        }
    }
    #[inline]
    fn consume_send_into(&self, take_send_target: &mut Vec<(i128,i128)>
                              , future_send: &mut Vec<i128>) -> bool {
        if let Some(ref send) = &self.send {
            //we only want to gab a max of LOCKED_CHANNEL_LENGTH_TO_COLLECTOR for this frame
            let mut buffer = [[0usize;TXL];config::CONSUMED_MESSAGES_BY_COLLECTOR +1];

            let count = {
                let mut tx_guard = bastion::run!(send.rx.lock());
                let tx = tx_guard.deref_mut();
                tx.take_slice( & mut buffer)
            };
            let populated_slice = &buffer[0..count];

            assert_eq!(future_send.len(),take_send_target.len());

            // each message, each details then each channel
            populated_slice.iter().for_each(|msg| {
                send.details.iter()
                    .zip(msg.iter())
                    .for_each(|(meta, val)| {
                        //consume any send left over from last frame
                        take_send_target[meta.id].1 += future_send[meta.id];
                        future_send[meta.id] = 0;//clear so it is ready for the take phase
                        //consume the new send values for this frame
                        take_send_target[meta.id].1 += *val as i128;
                    });
            });
            count > 0
        } else {
            false
        }
    }



}



impl <const LENGTH: usize> SteadyTelemetrySend<LENGTH> {

    pub fn new(tx: Arc<Mutex<Tx<[usize; LENGTH]>>>,
               count: [usize; LENGTH],
               inverse_local_index: [usize; LENGTH],
               start_now: Instant
    ) -> SteadyTelemetrySend<LENGTH> {
        SteadyTelemetrySend{ tx
            , count
            , last_telemetry_error: start_now
            , inverse_local_index
        }
    }

    pub(crate) fn process_event(&mut self, index: usize, id: usize, done: usize) -> usize {
        let telemetry = self;
        if index < MONITOR_NOT {
            telemetry.count[index] = telemetry.count[index].saturating_add(done);
            index
        } else if index == MONITOR_UNKNOWN {
            let local_index = find_my_index(telemetry, id);
            if local_index < MONITOR_NOT {
                telemetry.count[local_index] = telemetry.count[local_index].saturating_add(done);
            }
            local_index
        } else {
            index
        }
    }

}

#[derive(Clone, Default, Debug)]
pub struct ActorMetaData {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
    pub(crate) avg_mcpu: bool,
    pub(crate) avg_work: bool,
    pub percentiles_mcpu: Vec<Percentile>,
    pub percentiles_work: Vec<Percentile>,
    pub std_dev_mcpu: Vec<StdDev>,
    pub std_dev_work: Vec<StdDev>,
    pub trigger_mcpu: Vec<(Trigger<MCPU>,AlertColor)>,
    pub trigger_work: Vec<(Trigger<Work>,AlertColor)>,
    pub refresh_rate_in_bits: u8,
    pub window_bucket_in_bits: u8,
    pub usage_review: bool,
}

//this struct is immutable once built and must never change again
#[derive(Clone, Default, Debug)]
pub struct ChannelMetaData {
    pub(crate) id: usize,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) capacity: usize,
    pub(crate) display_labels: bool,
    pub(crate) line_expansion: bool,
    pub(crate) show_type: Option<&'static str>,
    pub(crate) refresh_rate_in_bits: u8,
    pub(crate) window_bucket_in_bits: u8,

    pub(crate) percentiles_filled: Vec<Percentile>, //each is a row
    pub(crate) percentiles_rate: Vec<Percentile>, //each is a row
    pub(crate) percentiles_latency: Vec<Percentile>, //each is a row

    pub(crate) std_dev_inflight: Vec<StdDev>, //each is a row
    pub(crate) std_dev_consumed: Vec<StdDev>, //each is a row
    pub(crate) std_dev_latency: Vec<StdDev>, //each is a row

    pub(crate) trigger_rate: Vec<(Trigger<Rate>,AlertColor)>, //if used base is green
    pub(crate) trigger_filled: Vec<(Trigger<Filled>,AlertColor)>, //if used base is green
    pub(crate) trigger_latency: Vec<(Trigger<Duration>,AlertColor)>, //if used base is green


    pub(crate) avg_filled: bool,
    pub(crate) avg_rate: bool,
    pub(crate) avg_latency: bool,

    pub(crate) connects_sidecar: bool,
    pub(crate) type_byte_count: usize,
    pub(crate) expects_to_be_monitored: bool,
}




pub trait RxTel : Send + Sync {
    //returns an iterator of usize channel ids
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;
    fn consume_actor(&self) -> Option<ActorStatus>;

    fn actor_metadata(&self) -> Arc<ActorMetaData>;


    fn consume_take_into(&self, take_send_source: &mut Vec<(i128,i128)>
                              , future_take: &mut Vec<i128>
                              , future_send: &mut Vec<i128>
                         ) -> bool;
    fn consume_send_into(&self, take_send_source: &mut Vec<(i128,i128)>
                              , future_send: &mut Vec<i128>) -> bool;


    fn actor_rx(&self) -> Option<Box<dyn RxDef>>;
}

#[cfg(test)]
pub(crate) mod monitor_tests {
    use std::ops::DerefMut;
    use async_std::test;
    use lazy_static::lazy_static;
    use std::sync::Once;
    use std::time::Duration;
    use futures_timer::Delay;
    use crate::{config, Graph, Rx, Tx, util};

    lazy_static! {
            static ref INIT: Once = Once::new();
    }

    //this is my unit test for relay_stats_tx_custom
    #[test]
    async fn test_relay_stats_tx_rx_custom() {
        util::logger::initialize();

        let mut graph = Graph::new("");
        let (tx_string, rx_string) = graph.channel_builder()
            .with_capacity(8)
            .build();

        let monitor = graph.new_test_monitor("test");
        let mut monitor = monitor.into_monitor([&rx_string], [&tx_string]);

        let mut rx_string_guard = rx_string.lock().await;
        let mut tx_string_guard = tx_string.lock().await;

        let rxd: &mut Rx<String> = rx_string_guard.deref_mut();
        let txd: &mut Tx<String> = tx_string_guard.deref_mut();


        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string(),false).await;
            count += 1;
        }

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_index], threshold);
        }

        monitor.relay_stats_smartly().await;

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_index], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_index], threshold);
        }
        Delay::new(Duration::from_micros(config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u64)).await;

        monitor.relay_stats_smartly().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_index], 0);
        }
    }

    #[test]
    async fn test_relay_stats_tx_rx_batch() {
        util::logger::initialize();

        let mut graph = Graph::new("");
        let monitor = graph.new_test_monitor("test");

        let (tx_string, rx_string) = graph.channel_builder().with_capacity(5).build();
        let mut monitor = monitor.into_monitor([&rx_string], [&tx_string]);

        let mut rx_string_guard = rx_string.lock().await;
        let mut tx_string_guard = tx_string.lock().await;

        let rxd: &mut Rx<String> = rx_string_guard.deref_mut();
        let txd: &mut Tx<String> = tx_string_guard.deref_mut();


        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string(),false).await;
            count += 1;
            if let Some(ref mut tx) = monitor.telemetry_send_tx {
                assert_eq!(tx.count[txd.local_index], count);
            }
            monitor.relay_stats_smartly().await;
        }

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_index], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Ok("test".to_string()));
            count -= 1;
        }
        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_index], threshold);
        }
        Delay::new(Duration::from_micros(config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u64)).await;

        monitor.relay_stats_smartly().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_index], 0);
        }
    }
}

pub(crate) fn find_my_index<const LEN:usize>(telemetry: &SteadyTelemetrySend<LEN>, goal: usize) -> usize {
    let (idx, _) = telemetry.inverse_local_index
        .iter()
        .enumerate()
        .find(|(_, &value)| value == goal)
        .unwrap_or( (MONITOR_NOT, &MONITOR_NOT));
    idx
}

impl <const RXL: usize, const TXL: usize> Drop for LocalMonitor<RXL, TXL> {
    //if possible we never want to loose telemetry data so we try to flush it out
    fn drop(&mut self) {
        bastion::run!(telemetry::setup::send_all_local_telemetry_async(self));
    }

}

pub struct LocalMonitor<const RX_LEN: usize, const TX_LEN: usize> {
    pub(crate) ident:                ActorIdentity,
    pub(crate) ctx:                  Option<Arc<bastion::context::BastionContext>>,
    pub(crate) telemetry_send_tx:    Option<SteadyTelemetrySend<TX_LEN>>,
    pub(crate) telemetry_send_rx:    Option<SteadyTelemetrySend<RX_LEN>>,
    pub(crate) telemetry_state:      Option<SteadyTelemetryActorSend>,
    pub(crate) last_telemetry_send:  Instant,
    pub(crate) runtime_state:        Arc<RwLock<GraphLiveliness>>,
    pub(crate) redundancy:           usize,


    #[cfg(test)]
    pub(crate) test_count: HashMap<&'static str, usize>,

}

///////////////////
impl <const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {


    /// Returns the unique identifier of the LocalMonitor instance.
    ///
    /// # Returns
    /// A `usize` representing the monitor's unique ID.
    pub fn id(&self) -> usize {
        self.ident.id
    }

    /// Retrieves the static name assigned to the LocalMonitor instance.
    ///
    /// # Returns
    /// A static string slice (`&'static str`) representing the monitor's name.
    pub fn name(&self) -> & 'static str {
        self.ident.name
    }

    /// Indicates the level of redundancy applied to the monitoring process.
    ///
    /// # Returns
    /// A `usize` value representing the redundancy level.
    pub fn redundancy(&self) -> usize {
        self.redundancy
    }

    /// Initiates a stop signal for the LocalMonitor, halting its monitoring activities.
    ///
    /// # Returns
    /// A `Result` indicating successful cessation of monitoring activities.
    pub async fn stop(&mut self) -> Result<(),()>  {
        if let Some(ref mut st) = self.telemetry_state {
            st.bool_stop = true;
            //TODO: probably rewrite to use the closed boolean on the channel.
        }// upon drop we will flush telemetry
        Ok(())
    }

    #[inline]
    pub fn is_running(&self, accept_fn: &mut dyn FnMut() -> bool) -> bool {
        match self.runtime_state.read() {
            Ok(liveliness) => {
                liveliness.is_running(self.ident, accept_fn )
            }
            Err(e) => {
                trace!("internal error,unable to get liveliness read lock {}",e);
                true //keep running as the default under error conditions
            }
        }
    }

    #[inline]
    pub fn request_graph_stop(&self) -> bool {
        match self.runtime_state.write() {
            Ok(mut liveliness) => {
                liveliness.request_shutdown();
                true
            }
            Err(e) => {
                trace!("internal error,unable to get liveliness write lock {}",e);
                false //keep running as the default under error conditions
            }
        }
    }

        #[inline]
    pub fn liveliness(&self) -> Arc<RwLock<GraphLiveliness>> {
        self.runtime_state.clone()
    }

    pub fn is_in_graph(&self) -> bool {
        self.ctx.is_some()
    }

    //testing
    async fn _async_yield_now() {
        let _ = pending::<()>().poll_unpin(&mut Context::from_waker(futures::task::noop_waker_ref()));
        // Immediately after polling, we return control, effectively yielding.
    }


    pub fn edge_simulator(&self) -> Option<EdgeSimulator> {
        self.ctx.as_ref().map(|ctx| EdgeSimulator::new(ctx.clone()))
    }


    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    /// Will hold the data if it is called more frequently than the collector can consume the data.
    /// This is designed for use in tight loops where telemetry data is collected frequently.
    ///
    /// # Asynchronous
    pub async fn relay_stats_smartly(&mut self) {
        //NOTE: not time ing this one as it is mine and internal
        telemetry::setup::try_send_all_local_telemetry(self).await;
    }

    /// Periodically relays telemetry data at a specified rate.
    ///
    /// # Parameters
    /// - `duration_rate`: The interval at which telemetry data should be sent.
    ///
    /// # Asynchronous
    pub async fn relay_stats_periodic(&mut self, duration_rate: Duration) {
        self.start_hot_profile(CALL_WAIT);
        //TODO: we should reuse teh Delay object in self
        //TODO: we may need a global clock for all the actors together to be more accurate.
        //   Warning: Delay can sometimes way many times longer than expected
        Delay::new(duration_rate.saturating_sub(self.last_telemetry_send.elapsed())).await;
        self.rollup_hot_profile();
        //this can not be measured since it sends the measurement of hot_profile.
        //also this is a special case where we do not want to measure the time it takes to send telemetry
        self.relay_stats_smartly().await;
    }

    /// Marks the start of a high-activity profile period for telemetry monitoring.
    ///
    /// # Parameters
    /// - `x`: The index representing the type of call being monitored.
    fn start_hot_profile(&mut self, x: usize) {
        if let Some(ref mut st) = self.telemetry_state {
            st.calls[x] = st.calls[x].saturating_add(1);
            if st.hot_profile.is_none() {
                st.hot_profile = Some(Instant::now())
            }
        };
    }

    /// Finalizes the current hot profile period, aggregating the collected telemetry data.
    fn rollup_hot_profile(&mut self) {
        if let Some(ref mut st) = self.telemetry_state {
            if let Some(d) = st.hot_profile.take() {
                st.await_ns_unit += Instant::elapsed(&d).as_nanos() as u64;
                assert!(st.instant_start.le(&d), "unit_start: {:?} call_start: {:?}", st.instant_start, d);
            }
        }
    }


    pub async fn wait_bundle_avail_units<T, const GIRTH: usize>(& mut self
                                                         , this: & SteadyRxBundle<T, GIRTH>
                                                         , avail_count: usize
                                                         , ready_channels: usize)
        where T: Send + Sync {

        self.start_hot_profile(CALL_OTHER);

        let futures = this.iter().map(|rx| {
            let rx = rx.clone();
            async move {
                let mut guard = rx.lock().await;
                guard.shared_wait_avail_units(avail_count).await;
            }
                .boxed() // Box the future to make them the same type
        });

        let mut futures: Vec<_> = futures.collect();

        let mut count_down = ready_channels.min(GIRTH);
        let mut futures = futures;

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, _index, remaining) = select_all(futures).await;
            futures = remaining;
            count_down -= 1;
            if 0 == count_down {
                break;
            }
        }

        self.rollup_hot_profile();

    }

    pub async fn wait_bundle_vacant_units<T, const GIRTH: usize>(& mut self
                                                                , this: & SteadyTxBundle<T, GIRTH>
                                                                , avail_count: usize
                                                                , ready_channels: usize)
        where T: Send + Sync {

        self.start_hot_profile(CALL_OTHER);

        let futures = this.iter().map(|tx| {
            let tx = tx.clone();
            async move {
                let mut guard = tx.lock().await;
                guard.shared_wait_vacant_units(avail_count).await;
            }
                .boxed() // Box the future to make them the same type
        });

        let mut futures: Vec<_> = futures.collect();

        let mut count_down = ready_channels.min(GIRTH);
        let mut futures = futures;

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, _index, remaining) = select_all(futures).await;
            futures = remaining;
            count_down -= 1;
            if 0 == count_down {
                break;
            }
        }

        self.rollup_hot_profile();

    }

    /// Attempts to peek at a slice of messages without removing them from the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance for peeking.
    /// - `elems`: A mutable slice to store the peeked messages.
    ///
    /// # Returns
    /// The number of messages peeked and stored in `elems`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    pub fn try_peek_slice<T>(& mut self, this: &mut Rx<T>, elems: &mut [T]) -> usize
        where T: Copy {
        this.shared_try_peek_slice(elems)
    }

    /// Asynchronously peeks at a slice of messages, waiting for a specified count to be available.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `wait_for_count`: The number of messages to wait for before peeking.
    /// - `elems`: A mutable slice to store the peeked messages.
    ///
    /// # Returns
    /// The number of messages peeked and stored in `elems`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    ///
    /// # Asynchronous
    pub async fn peek_async_slice<T>(&mut self, this: &mut Rx<T>, wait_for_count: usize, elems: &mut [T]) -> usize
    where T: Copy {
        this.shared_peek_async_slice(wait_for_count,elems).await
    }

    /// Retrieves and removes a slice of messages from the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `slice`: A mutable slice where the taken messages will be stored.
    ///
    /// # Returns
    /// The number of messages actually taken and stored in `slice`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    pub fn take_slice<T>(&mut self, this: & mut Rx<T>, slice: &mut [T]) -> usize
    where T: Copy {
        if let Some(ref mut st) = self.telemetry_state {
            st.calls[CALL_BATCH_READ] = st.calls[CALL_BATCH_READ].saturating_add(1);
        }
        let done = this.shared_take_slice(slice);
        this.local_index = if let Some(ref mut tel)= self.telemetry_send_rx {
            tel.process_event(this.local_index, this.channel_meta_data.id, done)
        } else {
            MONITOR_NOT
        };
        done
    }

    /// Attempts to take a single message from the channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` if a message is available, or `None` if the channel is empty.
    pub fn try_take<T>(&mut self, this: & mut Rx<T>) -> Option<T> {
        if let Some(ref mut st) = self.telemetry_state {
            st.calls[CALL_SINGLE_READ]=st.calls[CALL_SINGLE_READ].saturating_add(1);
        }
        match this.shared_try_take() {
            Some(msg) => {
                this.local_index = if let Some(ref mut tel)= self.telemetry_send_rx {
                    tel.process_event(this.local_index, this.channel_meta_data.id, 1)
                } else {
                    MONITOR_NOT
                };
                Some(msg)
            },
            None => {None}
        }
    }

    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message is available, or `None` if the channel is empty.
    pub fn try_peek<'a,T>(&'a mut self, this: &'a mut Rx<T>) -> Option<&T> {
        this.shared_try_peek()
    }

    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    pub fn try_peek_iter<'a,T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a {
        this.shared_try_peek_iter()
    }

    /// Asynchronously returns an iterator over the messages in the channel,
    /// waiting for a specified number of messages to be available.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `wait_for_count`: The number of messages to wait for before returning the iterator.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Asynchronous
    pub async fn peek_async_iter<'a,T>(&'a mut self, this: &'a mut Rx<T>, wait_for_count: usize) -> impl Iterator<Item = &'a T> + 'a {
        self.start_hot_profile(CALL_OTHER);
        let result = this.shared_peek_async_iter(wait_for_count).await;
        self.rollup_hot_profile();
        result
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    pub fn is_empty<T>(& mut self, this: & mut Rx<T>) -> bool {
        this.shared_is_empty()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    pub fn avail_units<T>(& mut self, this: & mut Rx<T>) -> usize {
        this.shared_avail_units()
    }

    /// Asynchronously waits for a specified duration.
    ///
    /// # Parameters
    /// - `duration`: The duration to wait.
    ///
    /// # Asynchronous
    pub async fn wait(& mut self, duration: Duration) {
        self.start_hot_profile(CALL_WAIT);
        Delay::new(duration).await;
        self.rollup_hot_profile();
    }

    /// Calls an asynchronous function and monitors its execution for telemetry.
    ///
    /// # Parameters
    /// - `f`: The asynchronous function to call.
    ///
    /// # Returns
    /// The output of the asynchronous function `f`.
    ///
    /// # Asynchronous
    pub async fn call_async<F>(&mut self, f: F) -> F::Output
        where F: Future {
        self.start_hot_profile(CALL_OTHER);
        let result = f.await;
        self.rollup_hot_profile();
        result
    }

    /// Asynchronously waits until a specified number of units are available in the Rx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `count`: The number of units to wait for availability.
    ///
    /// # Asynchronous
    pub async fn wait_avail_units<T>(&mut self, this: & mut Rx<T>, count:usize) {
        self.start_hot_profile(CALL_OTHER);
        let result = this.shared_wait_avail_units(count).await;
        self.rollup_hot_profile();
        result
    }

    /// Asynchronously peeks at the next available message in the channel without removing it.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message becomes available, or `None` if the channel is closed.
    ///
    /// # Asynchronous
    pub async fn peek_async<'a,T>(&'a mut self, this: &'a mut Rx<T>) -> Option<&T> {
        self.start_hot_profile(CALL_OTHER);
        let result = this.shared_peek_async().await;
        self.rollup_hot_profile();
        result
    }

    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `Result<T, String>`, where `Ok(T)` is the message if available, and `Err(String)` contains an error message if the retrieval fails.
    ///
    /// # Asynchronous
    pub async fn take_async<T>(& mut self, this: & mut Rx<T>) -> Result<T, String> {
        self.start_hot_profile(CALL_SINGLE_READ);
        let result = this.shared_take_async().await;
        self.rollup_hot_profile();
        match result {
            Ok(result) => {
                this.local_index = if let Some(ref mut tel)= self.telemetry_send_rx {
                    tel.process_event(this.local_index, this.channel_meta_data.id, 1)
                } else {
                    MONITOR_NOT
                };
                #[cfg(test)]
                self.test_count.entry("take_async").and_modify(|e| *e += 1).or_insert(1);
                Ok(result)
            },
            Err(error_msg) => {
                error!("Unexpected error take_async: {} {}", error_msg, self.ident.name);
                Err(error_msg)
            }
        }
    }

    /// Sends a slice of messages to the Tx channel until it is full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `slice`: A slice of messages to be sent.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    pub fn send_slice_until_full<T>(&mut self, this: & mut Tx<T>, slice: &[T]) -> usize
        where T: Copy {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[CALL_BATCH_WRITE]=st.calls[CALL_BATCH_WRITE].saturating_add(1);
        }

        let done = this.send_slice_until_full(slice);

        this.local_index = if let Some(ref mut tel)= self.telemetry_send_tx {
            tel.process_event(this.local_index, this.channel_meta_data.id, done)
        } else {
            MONITOR_NOT
        };

        done
    }

    /// Sends messages from an iterator to the Tx channel until it is full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `iter`: An iterator that yields messages of type `T`.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    pub fn send_iter_until_full<T,I: Iterator<Item = T>>(&mut self, this: & mut Tx<T>, iter: I) -> usize {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[CALL_BATCH_WRITE]=st.calls[CALL_BATCH_WRITE].saturating_add(1);
        }

        let done = this.send_iter_until_full(iter);

        this.local_index = if let Some(ref mut tel)= self.telemetry_send_tx {
            tel.process_event(this.local_index, this.channel_meta_data.id, done)
        } else {
            MONITOR_NOT
        };

        done
    }

    /// Attempts to send a single message to the Tx channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates successful send and `Err(T)` returns the message if the channel is full.
    pub fn try_send<T>(& mut self, this: & mut Tx<T>, msg: T) -> Result<(), T> {

        if let Some(ref mut st) = self.telemetry_state {
            st.calls[CALL_SINGLE_WRITE]=st.calls[CALL_SINGLE_WRITE].saturating_add(1);
        }

        match this.try_send(msg) {
            Ok(_) => {

                this.local_index = if let Some(ref mut tel)= self.telemetry_send_tx {
                    tel.process_event(this.local_index, this.channel_meta_data.id, 1)
                } else {
                    MONITOR_NOT
                };
                Ok(())
            },
            Err(sensitive) => {
                error!("Unexpected error try_send  telemetry: {} type: {}"
                    , self.ident.name, type_name::<T>());
                Err(sensitive)
            }
        }
    }

    /// Checks if the Tx channel is currently full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    pub fn is_full<T>(& mut self, this: & mut Tx<T>) -> bool {
        this.is_full()
    }

    /// Returns the number of vacant units in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    pub fn vacant_units<T>(& mut self, this: & mut Tx<T>) -> usize {
        this.shared_vacant_units()
    }

    /// Asynchronously waits until at least a specified number of units are vacant in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Asynchronous
    pub async fn wait_vacant_units<T>(& mut self, this: & mut Tx<T>, count:usize) {
        self.start_hot_profile(CALL_WAIT);
        let response = this.shared_wait_vacant_units(count).await;
        self.rollup_hot_profile();
        response
    }

    /// Asynchronously waits until the Tx channel is empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Asynchronous
    pub async fn wait_empty<T>(& mut self, this: & mut Tx<T>) {
        self.start_hot_profile(CALL_WAIT);
        let response = this.shared_wait_empty().await;
        self.rollup_hot_profile();
        response
    }

    /// Sends a message to the Tx channel asynchronously, waiting if necessary until space is available.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `a`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates that the message was successfully sent, and `Err(T)` if the send operation could not be completed.
    ///
    /// # Asynchronous
    pub async fn send_async<T>(& mut self, this: & mut Tx<T>, a: T, saturation_ok: bool) -> Result<(), T> {
       // TODO: instead of boolean we should have more descriptive enum.
        //      IgnoreAndWait, IgnoreAndErr, Warn (default), IgnoreInRelease
        self.start_hot_profile(CALL_SINGLE_WRITE);
       let result = this.shared_send_async(a, self.ident, saturation_ok).await;
       self.rollup_hot_profile();
       match result  {
           Ok(_) => {
               this.local_index = if let Some(ref mut tel)= self.telemetry_send_tx {
                   tel.process_event(this.local_index, this.channel_meta_data.id, 1)
               } else {
                   MONITOR_NOT
               };
               Ok(())
           },
           Err(sensitive) => {
               error!("Unexpected error send_async telemetry: {} type: {}", self.ident.name, type_name::<T>());
               Err(sensitive)
           }
       }
    }

}
