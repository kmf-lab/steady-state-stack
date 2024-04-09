use std::any::{type_name};

#[cfg(test)]
use std::collections::HashMap;

use std::ops::*;
use std::time::{Duration, Instant};

use std::sync::{Arc, RwLock};
use futures::lock::Mutex;
#[allow(unused_imports)]
use log::*; //allowed for all modules
use num_traits::{One, Zero};
use futures::future::select_all;

use futures_timer::Delay;
use std::future::Future;

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, Ordering};
use futures::channel::oneshot;

use futures::FutureExt;
use futures_util::select;
use crate::{AlertColor, config, GraphLivelinessState, MONITOR_NOT, MONITOR_UNKNOWN, Rx, RxBundle, RxDef, SendSaturation, StdDev, SteadyRx, SteadyTx, telemetry, Trigger, Tx, TxBundle};
use crate::actor_builder::{MCPU, Percentile, Work};
use crate::channel_builder::{Filled, Rate};
use crate::graph_liveliness::{ActorIdentity, GraphLiveliness};
use crate::graph_testing::{SideChannel, SideChannelResponder};
use crate::yield_now::yield_now;


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

    pub(crate) instant_start: Instant,
    pub(crate) count_restarts: Arc<AtomicU32>,
    pub(crate) bool_stop: bool,

    //move these 3 into a cell so we can do running counts with requiring mut
    pub(crate) await_ns_unit:          AtomicU64,

    pub(crate) hot_profile:            AtomicU64,
    pub(crate) hot_profile_concurrent: AtomicU16,

    pub(crate) calls:             [AtomicU16; 6],

}

impl SteadyTelemetryActorSend {

    pub(crate) fn status_reset(&mut self) {

        //ok on shutdown. confirm we are shutting down.
        //assert!(self.hot_profile.is_none(),"internal error");

        self.await_ns_unit = AtomicU64::new(0);
        self.instant_start = Instant::now();
        self.calls.iter().for_each(|f| f.store(0,Ordering::Relaxed));
    }

    pub(crate) fn status_message(&self) -> ActorStatus {

            let total_ns = self.instant_start.elapsed().as_nanos() as u64;

            //ok on shutdown.
            //assert!(self.hot_profile.is_none(),"internal error");

            assert!(total_ns>=self.await_ns_unit.load(Ordering::Relaxed),"should be: {} >= {}",total_ns,self.await_ns_unit.load(Ordering::Relaxed));

            let calls:Vec<u16> = self.calls.iter().map(|f|f.load(Ordering::Relaxed) ).collect();
            let calls:[u16;6] = calls.try_into().unwrap_or([0u16;6]);

            ActorStatus {
                total_count_restarts: self.count_restarts.load(Ordering::Relaxed),
                bool_stop: self.bool_stop,
                await_total_ns: self.await_ns_unit.load(Ordering::Relaxed),
                unit_total_ns: total_ns,
                calls,
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

    fn is_empty_and_closed(&self) -> bool {
        if let Some(ref take) = &self.take {
            let mut rx = bastion::run!(take.rx.lock());
            rx.is_empty() && rx.is_closed()
        } else {
            false
        }
    }
    
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

#[derive(Debug)]
pub struct TxMetaData(pub(crate) Arc<ChannelMetaData>);
#[derive(Debug)]
pub struct RxMetaData(pub(crate) Arc<ChannelMetaData>);




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

    fn is_empty_and_closed(&self)  -> bool;

}

#[cfg(test)]
pub(crate) mod monitor_tests {
    use std::ops::DerefMut;
    use async_std::test;
    use lazy_static::lazy_static;
    use std::sync::Once;
    use std::time::Duration;
    use futures_timer::Delay;
    use crate::{config, Graph, Rx, SendSaturation, Tx, util};

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

        let mut rxd = rx_string.lock().await;
        let mut txd = tx_string.lock().await;

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(&mut txd, "test".to_string(),SendSaturation::Warn).await;
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
            let x = monitor.take_async(&mut rxd).await;
            assert_eq!(x, Some("test".to_string()));
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
            let _ = monitor.send_async(txd, "test".to_string(),SendSaturation::Warn).await;
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
            assert_eq!(x, Some("test".to_string()));
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
    pub(crate) last_telemetry_send:  Instant, //NOTE: we use mutable for counts so no need for Atomic here
    pub(crate) last_perodic_wait:    AtomicU64,
    pub(crate) runtime_state:        Arc<RwLock<GraphLiveliness>>,
    pub(crate) oneshot_shutdown:     Arc<Mutex<oneshot::Receiver<()>>>,
    pub(crate) actor_start_time:     Instant, //never changed from context
    pub(crate) node_tx_rx:           Option<Arc<Mutex<SideChannel>>>,

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



    #[inline]
    pub fn is_running(&self, accept_fn: &mut dyn FnMut() -> bool) -> bool {
        match self.runtime_state.read() {
            Ok(liveliness) => {
                liveliness.is_running(self.ident, accept_fn )
            }
            Err(e) => {
                error!("internal error,unable to get liveliness read lock {}",e);
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

    pub(crate) fn is_liveliness_in(&self, target: &[GraphLivelinessState], upon_posion: bool) -> bool {
        match self.runtime_state.read() {
            Ok(liveliness) => {
                liveliness.is_in_state(target)
            }
            Err(e) => {
                trace!("internal error,unable to get liveliness read lock {}",e);
                upon_posion
            }
        }
    }



    pub fn wait_while_running(&self) -> impl Future<Output = Result<(), ()>> {
        crate::graph_liveliness::WaitWhileRunningFuture::new(self.runtime_state.clone())
    }


    pub fn is_in_graph(&self) -> bool {
        self.ctx.is_some()
    }




    pub fn sidechannel_responder(&self) -> Option<SideChannelResponder> {
        //if we have no back channel plane then we can not simulate the edges
        self.node_tx_rx.as_ref().map(|node_tx_rx| SideChannelResponder::new(node_tx_rx.clone() ))
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

    /// Asynchronously waits for a specified duration.
    ///
    /// # Parameters
    /// - `duration`: The duration to wait.
    ///
    /// # Asynchronous
    pub async fn wait(&self, duration: Duration) {
        self.start_hot_profile(CALL_WAIT);
        let one_down = &mut self.oneshot_shutdown.lock().await;
        select! { _ = one_down.deref_mut() => {}, _ =Delay::new(duration).fuse() => {} }
        self.rollup_hot_profile();
    }

    /// yield so other actors may be able to make use of this thread. Returns
    /// immediately if there is nothing scheduled to check.
    pub async fn yield_now(&self) {
        self.start_hot_profile(CALL_WAIT); //start timer to measure duration
        yield_now().await;
        self.rollup_hot_profile(); //rollup the duration into the totals
    }

    /// Periodically relays telemetry data at a specified rate.
    ///
    /// # Parameters
    /// - `duration_rate`: The interval at which telemetry data should be sent.
    ///
    /// # Asynchronous
    pub async fn relay_stats_periodic(&mut self, duration_rate: Duration) -> bool {
        let result = self.wait_periodic(duration_rate).await;
        self.relay_stats_smartly().await;
        result
    }

    /// Waits for a specified duration, ensuring a consistent periodic interval between calls.
    ///
    /// This method helps maintain a consistent period between consecutive calls, even if the
    /// execution time of the work performed in between calls fluctuates. It calculates the
    /// remaining time until the next desired periodic interval and waits for that duration.
    ///
    /// If a shutdown signal is detected during the waiting period, the method returns early
    /// with a value of `false`. Otherwise, it waits for the full duration and returns `true`.
    ///
    /// # Arguments
    ///
    /// * `duration_rate` - The desired duration between periodic calls.
    ///
    /// # Returns
    ///
    /// * `true` if the full waiting duration was completed without interruption.
    /// * `false` if a shutdown signal was detected during the waiting period.
    ///
    pub async fn wait_periodic(&self, duration_rate: Duration) -> bool {

        let one_down = &mut self.oneshot_shutdown.lock().await;

        let now_nanos = self.actor_start_time.elapsed().as_nanos() as u64;
        let run_duration = now_nanos - self.last_perodic_wait.load(Ordering::Relaxed);
        let remaining_duration = duration_rate.saturating_sub( Duration::from_nanos(run_duration) );

        let mut operation = &mut Delay::new(remaining_duration).fuse();

        let result = select! {
                _= &mut one_down.deref_mut() => false,
                _= operation => true
        };
        self.last_perodic_wait.store(remaining_duration.as_nanos() as u64 + now_nanos, Ordering::Relaxed);
        result

    }

    /// Marks the start of a high-activity profile period for telemetry monitoring.
    ///
    /// # Parameters
    /// - `x`: The index representing the type of call being monitored.
    pub(crate) fn start_hot_profile(&self, x: usize) {
        if let Some(ref st) = self.telemetry_state {
            let _ = st.calls[x].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
            //if you are the first then store otherwise we leave it as the oldest start
            if st.hot_profile_concurrent.fetch_add(1,Ordering::Relaxed).is_zero() {
                st.hot_profile.store(self.actor_start_time.elapsed().as_nanos() as u64
                                     , Ordering::Relaxed);
            }
        };
    }

    /// Finalizes the current hot profile period, aggregating the collected telemetry data.
    pub(crate) fn rollup_hot_profile(&self) {
        if let Some(ref st) = self.telemetry_state {
            if st.hot_profile_concurrent.fetch_sub(1,Ordering::Relaxed).is_one() {
                let prev = st.hot_profile.load(Ordering::Relaxed);
                let _ = st.await_ns_unit.fetch_update(Ordering::Relaxed,Ordering::Relaxed
                                  ,|f|
                                      Some((f+self.actor_start_time.elapsed().as_nanos() as u64).saturating_sub(prev)));
            }
        }
    }



    pub async fn wait_avail_units_bundle<T>(& self
                                            , this: &mut RxBundle<'_, T>
                                            , avail_count: usize
                                            , ready_channels: usize) -> bool
        where T: Send + Sync {

        self.start_hot_profile(CALL_OTHER);

        let mut count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));
        let futures = this.iter_mut().map(|rx| {
            let local_r = result.clone();
            async move {
                let bool_result = rx.shared_wait_avail_units(avail_count).await;
                if !bool_result {
                    local_r.store(false,Ordering::Relaxed);
                }
            }.boxed() // Box the future to make them the same type
        });

        let futures: Vec<_> = futures.collect();

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
        result.load(Ordering::Relaxed)

    }

    pub async fn wait_vacant_units_bundle<T>(& self
                                             , this: &mut TxBundle<'_, T>
                                             , avail_count: usize
                                             , ready_channels: usize) -> bool
        where T: Send + Sync {

        self.start_hot_profile(CALL_OTHER);
        let mut count_down = ready_channels.min(this.len());

        let result = Arc::new(AtomicBool::new(true));
        let futures = this.iter_mut().map(|tx| {
            let local_r = result.clone();
            async move {
                let bool_result = tx.shared_wait_vacant_units(avail_count).await;
                if !bool_result {
                    local_r.store(false,Ordering::Relaxed);
                }
            }
                .boxed() // Box the future to make them the same type
        });
        let futures: Vec<_> = futures.collect();
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
        result.load(Ordering::Relaxed)
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
    pub fn try_peek_slice<T>(self, this: &mut Rx<T>, elems: &mut [T]) -> usize
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
    pub async fn peek_async_slice<T>(&self, this: &mut Rx<T>, wait_for_count: usize, elems: &mut [T]) -> usize
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
        if let Some(ref st) = self.telemetry_state {
            let _ = st.calls[CALL_BATCH_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
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
        if let Some(ref st) = self.telemetry_state {
            let _= st.calls[CALL_SINGLE_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
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
    pub fn try_peek<'a,T>(&'a self, this: &'a mut Rx<T>) -> Option<&T> {
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
    pub async fn peek_async_iter<'a,T>(&'a self, this: &'a mut Rx<T>, wait_for_count: usize) -> impl Iterator<Item = &'a T> + 'a {
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
    pub fn is_empty<T>(& self, this: & mut Rx<T>) -> bool {
        this.shared_is_empty()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    pub fn avail_units<T>(& self, this: & mut Rx<T>) -> usize {
        this.shared_avail_units()
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
    pub async fn call_async<F>(&self, operation: F) -> Option<F::Output>
      where F: Future {
        self.start_hot_profile(CALL_OTHER);
        let one_down = &mut self.oneshot_shutdown.lock().await;
        let result = select! { _ = one_down.deref_mut() => None, r = operation.fuse() => Some(r), };
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
    pub async fn wait_avail_units<T>(& self, this: & mut Rx<T>, count:usize) -> bool {
        self.start_hot_profile(CALL_OTHER);
        let ok = this.shared_wait_avail_units(count).await;
        self.rollup_hot_profile();
        ok

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
    pub async fn peek_async<'a,T>(&'a self, this: &'a mut Rx<T>) -> Option<&T> {
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
    pub async fn take_async<T>(&mut self, this: & mut Rx<T>) -> Option<T> {
        self.start_hot_profile(CALL_SINGLE_READ);
        let result = this.shared_take_async().await;
        self.rollup_hot_profile();
        match result {
            Some(result) => {
                this.local_index = if let Some(ref mut tel)= self.telemetry_send_rx {
                    tel.process_event(this.local_index, this.channel_meta_data.id, 1)
                } else {
                    MONITOR_NOT
                };
                #[cfg(test)]
                self.test_count.entry("take_async").and_modify(|e| *e += 1).or_insert(1);
                Some(result)
            },
            None => {
                //this case is expected if a shutdown signal was detected.
                None
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
            let _= st.calls[CALL_BATCH_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
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
            let _ = st.calls[CALL_BATCH_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
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
    pub fn try_send<T>(&mut self, this: & mut Tx<T>, msg: T) -> Result<(), T> {

        if let Some(ref mut st) = self.telemetry_state {
            let _ = st.calls[CALL_SINGLE_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
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
    pub fn is_full<T>(& self, this: & mut Tx<T>) -> bool {
        this.is_full()
    }

    /// Returns the number of vacant units in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    pub fn vacant_units<T>(& self, this: & mut Tx<T>) -> usize {
        this.shared_vacant_units()
    }

    /// Asynchronously waits until at least a specified number of units are vacant in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Asynchronous
    pub async fn wait_vacant_units<T>(&self, this: & mut Tx<T>, count:usize) -> bool {
        self.start_hot_profile(CALL_WAIT);
        let ok = this.shared_wait_vacant_units(count).await;
        self.rollup_hot_profile();
        ok
    }


    /// Asynchronously waits until the Tx channel is empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Asynchronous
    pub async fn wait_empty<T>(& self, this: & mut Tx<T>) -> bool {
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
    pub async fn send_async<T>(&mut self, this: & mut Tx<T>, a: T, saturation: SendSaturation) -> Result<(), T> {

       self.start_hot_profile(CALL_SINGLE_WRITE);
       let result = this.shared_send_async(a, self.ident, saturation).await;
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
