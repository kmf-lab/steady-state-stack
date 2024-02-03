use std::ops::{DerefMut, Sub};
use std::time::{Duration, Instant};
use bastion::run;
use std::sync::Arc;
use futures::lock::Mutex;
use num_traits::Zero;
use crate::{config, MONITOR_NOT, MONITOR_UNKNOWN, Percentile, Rx, StdDev, Trigger, Tx};
use crate::config::MAX_TELEMETRY_ERROR_RATE_SECONDS;

pub struct SteadyTelemetryRx<const RXL: usize, const TXL: usize> {
    pub(crate) send: Option<SteadyTelemetryTake<TXL>>,
    pub(crate) take: Option<SteadyTelemetryTake<RXL>>,
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
    fn consume_take_into(&self, send_source: &mut Vec<i128>, take_target: &mut Vec<i128>, future_target: &mut Vec<i128>) {
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

            //first pull the data from last frame into here
            take.details.iter().for_each(|meta| {
                take_target[meta.id] += future_target[meta.id];
                future_target[meta.id] = 0;
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
        }
    }
    #[inline]
    fn consume_send_into(&self, send_target: &mut Vec<i128>) {
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
    pub(crate) tx: Arc<Mutex<Tx<[usize; LENGTH]>>>,
    pub(crate) count: [usize; LENGTH],
    pub(crate) last_telemetry_error: Instant,
    pub(crate) actor_name: &'static str,
    pub(crate) inverse_local_index: [usize; LENGTH],
}

impl <const LENGTH: usize> SteadyTelemetrySend<LENGTH> {

    pub fn actor_name(&self) -> &'static str {
        self.actor_name
    }

    pub fn new(tx: Arc<Mutex<Tx<[usize; LENGTH]>>>,
               count: [usize; LENGTH],
               actor_name: &'static str,
               inverse_local_index: [usize; LENGTH],
    ) -> SteadyTelemetrySend<LENGTH> {
        SteadyTelemetrySend{ tx
            , count
            , last_telemetry_error: Instant::now().sub(Duration::from_secs(1+MAX_TELEMETRY_ERROR_RATE_SECONDS as u64))
            , actor_name
            , inverse_local_index
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ChannelMetaData {
    pub(crate) id: usize,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) capacity: usize,
    pub(crate) display_labels: bool,
    pub(crate) line_expansion: bool,
    pub(crate) show_type: Option<&'static str>,
    pub(crate) window_bucket_in_bits: u8, //for percentiles and ma
    pub(crate) percentiles_inflight: Vec<Percentile>, //each is a row
    pub(crate) percentiles_consumed: Vec<Percentile>, //each is a row
    pub(crate) std_dev_inflight: Vec<StdDev>, //each is a row
    pub(crate) std_dev_consumed: Vec<StdDev>, //each is a row
    pub(crate) red: Vec<Trigger>, //if used base is green
    pub(crate) yellow: Vec<Trigger>, //if used base is green
    pub(crate) avg_inflight: bool,
    pub(crate) avg_consumed: bool,
    pub(crate) connects_sidecar: bool,
}

pub struct SteadyTelemetryTake<const LENGTH: usize> {
    pub(crate) rx: Arc<Mutex<Rx<[usize; LENGTH]>>>,
    pub(crate) details: Vec<Arc<ChannelMetaData>>,
}


pub trait RxTel : Send + Sync {


    //returns an iterator of usize channel ids
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    fn consume_take_into(&self, send_source: &mut Vec<i128>, take_target: &mut Vec<i128>, future_target: &mut Vec<i128>);
    fn consume_send_into(&self, send_target: &mut Vec<i128>);

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

        let mut graph = Graph::new();
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

        let mut graph = Graph::new();
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
        .unwrap_or_else(|| (MONITOR_NOT, &MONITOR_NOT));
    idx
}