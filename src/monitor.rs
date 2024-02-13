use std::ops::*;
use std::time::{Instant};
use bastion::run;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use futures::lock::Mutex;
use num_traits::Zero;
use crate::{config, MONITOR_NOT, MONITOR_UNKNOWN, Percentile, Rx, StdDev, Trigger, Tx};
use crate::channel_builder::{SteadyRx, SteadyTx};

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

    pub(crate) fn status_message(&self) -> Option<ActorStatus> {

        if config::TELEMETRY_FOR_ACTORS {
            let total_ns = self.instant_start.elapsed().as_nanos() as u64;

            //ok on shutdown.
            //assert!(self.hot_profile.is_none(),"internal error");

            assert!(total_ns>=self.await_ns_unit,"should be: {} >= {}",total_ns,self.await_ns_unit);
            Some(ActorStatus {
                total_count_restarts: self.count_restarts.load(Ordering::Relaxed),
                bool_stop: self.bool_stop,
                await_total_ns: self.await_ns_unit,
                unit_total_ns: total_ns,
                redundancy: self.redundancy,
                calls: self.calls,
            })
        } else {
            None
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


    fn consume_actor(&self) -> Option<ActorStatus> {
        if let Some(ref act) = &self.actor {
            let mut buffer = [ActorStatus::default();config::LOCKED_CHANNEL_LENGTH_TO_COLLECTOR+1];
            let count = {
                let mut guard = run!(act.lock());
                let act = guard.deref_mut();
                act.take_slice( & mut buffer)
            };

            let mut await_total_ns:       u64 = 0;
            let mut unit_total_ns:        u64 = 0;
            let mut single_read_calls:    u16 = 0;
            let mut batch_read_calls:     u16 = 0;
            let mut single_write_calls:   u16 = 0;
            let mut batch_write_calls:    u16 = 0;
            let mut other_calls:          u16 = 0;
            let mut wait_calls:           u16 = 0;

            //let populated_slice = &buffer[0..count];
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
    fn consume_take_into(&self, send_source: &mut Vec<i128>, take_target: &mut Vec<i128>, future_target: &mut Vec<i128>) -> bool {
        if let Some(ref take) = &self.take {
            let mut buffer = [[0usize;RXL];config::LOCKED_CHANNEL_LENGTH_TO_COLLECTOR+1];

            let count = {
                let mut rx_guard = run!(take.rx.lock());
                let rx = rx_guard.deref_mut();
                rx.take_slice( & mut buffer)
            };
            let populated_slice = &buffer[0..count];

            //this logic is key because we must find the break point between frames.
            //we use send as a boundary to know that these takes on the same channel
            //ether belong to this frame or the next frame.  Data from the last frame
            //is then used to start our new target data.

            //this is essential to ensure send>=take for all frames before we generate labels
            //this also ensures the data is highly compressable

            //first pull the data we can from last frame into here
            take.details.iter().for_each(|meta| {
                //note we may be spanning the boundary of a failed actor
                //if that is the case we may not be able to pick up all the data
                //and must leave some for future frames
                let max_takeable = send_source[meta.id]-take_target[meta.id];
                assert!(max_takeable.ge(&0),"internal error");
                let value_taken = max_takeable.min(future_target[meta.id]);
                take_target[meta.id] += value_taken;
                future_target[meta.id] -= value_taken;
            });

            //pull in new data for this frame
            populated_slice.iter().for_each(|msg| {
                take.details.iter()
                    .zip(msg.iter())
                    .for_each(|(meta, val)| {
                        let limit = send_source[meta.id];
                        let val = *val as i128;
                        //we can go up to the limit but no more
                        //once we hit the limit we start putting the data into the future
                        if i128::is_zero(&future_target[meta.id]) && val+take_target[meta.id] <= limit {
                            take_target[meta.id] += val;
                        } else {
                            future_target[meta.id] += val;
                        }
                    });
            });
            count>0
        } else {
            false
        }
    }
    #[inline]
    fn consume_send_into(&self, send_target: &mut Vec<i128>) -> bool {
        if let Some(ref send) = &self.send {
            //we only want to gab a max of LOCKED_CHANNEL_LENGTH_TO_COLLECTOR for this frame
            let mut buffer = [[0usize;TXL];config::LOCKED_CHANNEL_LENGTH_TO_COLLECTOR+1];

            let count = {
                let mut tx_guard = run!(send.rx.lock());
                let tx = tx_guard.deref_mut();
                tx.take_slice( & mut buffer)
            };
            let populated_slice = &buffer[0..count];

            // each message, each details then each channel
            populated_slice.iter().for_each(|msg| {
                send.details.iter()
                    .zip(msg.iter())
                    .for_each(|(meta, val)| {
                        send_target[meta.id] += *val as i128;
                    });
            });
            count > 0
        } else {
            false
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
}

#[derive(Clone, Default)]
pub struct ActorMetaData {
    pub(crate) avg_mcpu: bool,
    pub(crate) avg_work: bool,

    pub percentiles_mcpu: Vec<Percentile>,
    pub percentiles_work: Vec<Percentile>,
    pub std_dev_mcpu: Vec<StdDev>,
    pub std_dev_work: Vec<StdDev>,
    pub red: Vec<Trigger>,
    pub yellow: Vec<Trigger>,
    pub refresh_rate_in_bits: u8,
    pub window_bucket_in_bits: u8,
    pub usage_review: bool,
}


#[derive(Clone, Default)]
pub struct ChannelMetaData {
    pub(crate) id: usize,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) capacity: usize,
    pub(crate) display_labels: bool,
    pub(crate) line_expansion: bool,
    pub(crate) show_type: Option<&'static str>,

    pub(crate) refresh_rate_in_bits: u8,
    pub(crate) window_bucket_in_bits: u8,

    pub(crate) percentiles_inflight: Vec<Percentile>, //each is a row
    pub(crate) percentiles_consumed: Vec<Percentile>, //each is a row
    pub(crate) percentiles_latency: Vec<Percentile>, //each is a row

    pub(crate) std_dev_inflight: Vec<StdDev>, //each is a row
    pub(crate) std_dev_consumed: Vec<StdDev>, //each is a row
    pub(crate) std_dev_latency: Vec<StdDev>, //each is a row


    pub(crate) red: Vec<Trigger>, //if used base is green
    pub(crate) yellow: Vec<Trigger>, //if used base is green

    pub(crate) avg_inflight: bool,
    pub(crate) avg_consumed: bool,
    pub(crate) avg_latency: bool,

    pub(crate) connects_sidecar: bool,
}




pub trait RxTel : Send + Sync {
    //returns an iterator of usize channel ids
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;
    fn consume_actor(&self) -> Option<ActorStatus>;

    fn actor_metadata(&self) -> Arc<ActorMetaData>;


    fn consume_take_into(&self, send_source: &mut Vec<i128>, take_target: &mut Vec<i128>, future_target: &mut Vec<i128>) -> bool;
    fn consume_send_into(&self, send_target: &mut Vec<i128>) -> bool;

    fn biggest_tx_id(&self) -> usize;
    fn biggest_rx_id(&self) -> usize;


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
        let mut rx_string_guard = rx_string.lock().await;
        let mut tx_string_guard = tx_string.lock().await;

        let rxd: &mut Rx<String> = rx_string_guard.deref_mut();
        let txd: &mut Tx<String> = tx_string_guard.deref_mut();

        let mut monitor = monitor.into_monitor(&[rxd], &[txd]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string()).await;
            count += 1;
        }

        if let Some(ref mut tx) = monitor.telemetry_send_tx {
            assert_eq!(tx.count[txd.local_index], threshold);
        }

        monitor.relay_stats_all().await;

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

        monitor.relay_stats_all().await;

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

        let mut rx_string_guard = rx_string.lock().await;
        let mut tx_string_guard = tx_string.lock().await;

        let rxd: &mut Rx<String> = rx_string_guard.deref_mut();
        let txd: &mut Tx<String> = tx_string_guard.deref_mut();

        let mut monitor = monitor.into_monitor(&[rxd], &[txd]);

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string()).await;
            count += 1;
            if let Some(ref mut tx) = monitor.telemetry_send_tx {
                assert_eq!(tx.count[txd.local_index], count);
            }
            monitor.relay_stats_all().await;
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

        monitor.relay_stats_all().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_index], 0);
        }
    }
}

pub(crate) fn process_event<const LEN:usize, F>(target: &mut Option<SteadyTelemetrySend<LEN>>
                                                , index: usize
                                                , id: usize
                                                , mut f: F) -> usize
    where F: FnMut( &mut SteadyTelemetrySend<{ LEN }>, usize), {
    if let Some(ref mut telemetry) = target {
        if index < MONITOR_NOT {
            f(telemetry, index);
            index
        } else if index == MONITOR_UNKNOWN {
            let local_index = find_my_index(telemetry, id);
            if local_index < MONITOR_NOT {
                f(telemetry, local_index);
            }
            local_index
        } else {
            index
        }
    } else {
        MONITOR_NOT
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