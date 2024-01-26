use std::ops::{DerefMut, Sub};
use std::time::{Duration, Instant};
use bastion::run;
use std::sync::Arc;
use futures::lock::Mutex;
use crate::steady_state::{ColorTrigger, config, MONITOR_NOT, MONITOR_UNKNOWN};
use crate::steady_state::Rx;
use crate::steady_state::Tx;
use crate::steady_state::config::MAX_TELEMETRY_ERROR_RATE_SECONDS;

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
    pub(crate) tx: Arc<Mutex<Tx<[usize; LENGTH]>>>,
    pub(crate) count: [usize; LENGTH],
    pub(crate) limits: [usize; LENGTH],
    pub(crate) last_telemetry_error: Instant,
    pub(crate) actor_name: &'static str,
    pub(crate) inverse_local_index: [usize; LENGTH],
}

impl <const LENGTH: usize> SteadyTelemetrySend<LENGTH> {
    pub fn new(tx: Arc<Mutex<Tx<[usize; LENGTH]>>>,
               count: [usize; LENGTH],
               limits: [usize; LENGTH],
               actor_name: &'static str,
               inverse_local_index: [usize; LENGTH],
    ) -> SteadyTelemetrySend<LENGTH> {
        SteadyTelemetrySend{
             tx,count,limits
            , last_telemetry_error: Instant::now().sub(Duration::from_secs(1+MAX_TELEMETRY_ERROR_RATE_SECONDS as u64))
            , actor_name
            , inverse_local_index
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
    pub(crate) red: Option<ColorTrigger>, //if used base is green
    pub(crate) yellow: Option<ColorTrigger>, //if used base is green
}

pub struct SteadyTelemetryTake<const LENGTH: usize> {
    pub(crate) rx: Arc<Mutex<Rx<[usize; LENGTH]>>>,
    pub(crate) details: Vec<Arc<ChannelMetaData>>,
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
    use crate::steady_state::*;
    use async_std::test;
    use lazy_static::lazy_static;
    use std::sync::Once;
    use std::time::Duration;
    use futures_timer::Delay;
    use crate::steady_state::Rx;
    use crate::steady_state::Tx;
    use crate::steady_state::Graph;
    lazy_static! {
            static ref INIT: Once = Once::new();
    }

    //this is my unit test for relay_stats_tx_custom
    #[test]
    async fn test_relay_stats_tx_rx_custom() {
        crate::steady_state::util::util_tests::initialize_logger();

        let mut graph = Graph::new();
        let (tx_string, rx_string) = graph.channel_builder(8)
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
        monitor.relay_stats_tx_set_custom_batch_limit(txd, threshold);
        monitor.relay_stats_batch().await;

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
        monitor.relay_stats_rx_set_custom_batch_limit(rxd, threshold);
        Delay::new(Duration::from_micros(config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u64)).await;

        monitor.relay_stats_batch().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_index], 0);
        }
    }

    #[test]
    async fn test_relay_stats_tx_rx_batch() {
        crate::steady_state::util::util_tests::initialize_logger();

        let mut graph = Graph::new();
        let monitor = graph.new_test_monitor("test");

        let (tx_string, rx_string) = graph.channel_builder(5).build();

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
            monitor.relay_stats_batch().await;
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

        monitor.relay_stats_batch().await;

        if let Some(ref mut rx) = monitor.telemetry_send_rx {
            assert_eq!(rx.count[rxd.local_index], 0);
        }
    }
}

pub(crate) fn process_event<const LEN:usize, F>(target: &mut Option<SteadyTelemetrySend<LEN>>
                                                , index: usize, id: usize, mut f: F) -> usize
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