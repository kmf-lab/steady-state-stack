use std::collections::VecDeque;
use std::ops::{Deref, DerefMut, Sub};
use std::process::exit;
use std::sync::{Arc};
use std::sync::atomic::{AtomicU16, AtomicU64};
use std::time::{Duration, Instant};
use log::*;
use num_traits::Zero;
use ringbuf::traits::Observer;
use crate::{config, Graph, GraphLivelinessState, MONITOR_NOT, MONITOR_UNKNOWN, SteadyContext, telemetry};
use crate::channel_builder::ChannelBuilder;
use crate::config::MAX_TELEMETRY_ERROR_RATE_SECONDS;
use crate::monitor::{ChannelMetaData, find_my_index, LocalMonitor, RxTel, SteadyTelemetryActorSend, SteadyTelemetryRx, SteadyTelemetrySend, SteadyTelemetryTake};
use crate::telemetry::{metrics_collector, metrics_server};
use crate::telemetry::metrics_collector::CollectorDetail;

pub(crate) fn construct_telemetry_channels<const RX_LEN: usize, const TX_LEN: usize>(that: &SteadyContext
                                                                                     , rx_meta_data: Vec<Arc<ChannelMetaData>>
                                                                                     , rx_inverse_local_idx: [usize; RX_LEN]
                                                                                     , tx_meta_data: Vec<Arc<ChannelMetaData>>
                                                                                     , tx_inverse_local_idx: [usize; TX_LEN]) -> (Option<SteadyTelemetrySend<{ RX_LEN }>>, Option<SteadyTelemetrySend<{ TX_LEN }>>, Option<SteadyTelemetryActorSend>) {
//NOTE: if this child telemetry is monitored so we will create the appropriate channels
    let start_now = Instant::now().sub(Duration::from_secs(1 + MAX_TELEMETRY_ERROR_RATE_SECONDS as u64));

    let channel_builder = ChannelBuilder::new(that.channel_count.clone()
                                              ,that.oneshot_shutdown_vec.clone())
        .with_labels(&["steady_state-telemetry"], false)
        .with_compute_refresh_window_bucket_bits(0, 0)
        .with_capacity(config::REAL_CHANNEL_LENGTH_TO_COLLECTOR);

    let rx_tuple: (Option<SteadyTelemetrySend<RX_LEN>>, Option<SteadyTelemetryTake<RX_LEN>>)
        = if 0usize == RX_LEN {
        (None, None)
    } else {
        let (telemetry_send_rx, telemetry_take_rx) = channel_builder.build();
        (Some(SteadyTelemetrySend::new(telemetry_send_rx, [0; RX_LEN], rx_inverse_local_idx, start_now), )
         , Some(SteadyTelemetryTake { rx: telemetry_take_rx, details: rx_meta_data }))
    };

    let tx_tuple: (Option<SteadyTelemetrySend<TX_LEN>>, Option<SteadyTelemetryTake<TX_LEN>>)
        = if 0usize == TX_LEN {
        (None, None)
    } else {
        let (telemetry_send_tx, telemetry_take_tx) = channel_builder.build();
        (Some(SteadyTelemetrySend::new(telemetry_send_tx, [0; TX_LEN], tx_inverse_local_idx, start_now), )
         , Some(SteadyTelemetryTake { rx: telemetry_take_tx, details: tx_meta_data }))
    };

    let act_tuple = channel_builder.build();
    let det = SteadyTelemetryRx {
        send: tx_tuple.1,
        take: rx_tuple.1,
        actor: Some(act_tuple.1),
        actor_metadata: that.actor_metadata.clone(),
    };
    let idx:Option<usize> = match that.all_telemetry_rx.read() {
        Ok(guard) => {
            //need to hand off to the collector
            let shared_vec = guard.deref();
            let index:Option<usize> = shared_vec.iter()
                .enumerate()
                .find(|(_,x)| x.ident == that.ident)
                .map(|(idx,_)|idx);
            index
        }
        Err(_) => {
            error!("internal error: unable to get read lock");
            None
        }
    };
    match that.all_telemetry_rx.write() {
        Ok(mut guard) => {
            let shared_vec = guard.deref_mut();
            if let Some(idx) = idx {
                //we add new SteadyTelemetryRx which waits for the old one to be consumed first
                shared_vec[idx].telemetry_take.push_back(Box::new(det));
            } else {
                let mut telemetry_take:VecDeque<Box<dyn RxTel>> = VecDeque::new();
                telemetry_take.push_back(Box::new(det));
                shared_vec.push(CollectorDetail {
                    ident: that.ident, telemetry_take
                }); //add new telemetry channels
            }
        }
        Err(_) => {
            error!("internal error: failed to write to all_telemetry_rx");
        }
    }

    let calls:[AtomicU16;6] = [AtomicU16::new(0),AtomicU16::new(0),AtomicU16::new(0),AtomicU16::new(0),AtomicU16::new(0),AtomicU16::new(0)];
    let telemetry_actor =
        Some(SteadyTelemetryActorSend {
            tx: act_tuple.0,
            last_telemetry_error: start_now,
            instant_start: Instant::now(),
            await_ns_unit: AtomicU64::new(0),
            hot_profile: AtomicU64::new(0),
            calls,
            count_restarts: that.count_restarts.clone(),
            bool_stop: false,
        });

    let telemetry_send_rx = rx_tuple.0;
    let telemetry_send_tx = tx_tuple.0;
    (telemetry_send_rx, telemetry_send_tx, telemetry_actor)
}


pub(crate) fn build_optional_telemetry_graph( graph: & mut Graph)
{


        if config::TELEMETRY_SERVER {

            let base = if config::SHOW_TELEMETRY_ON_TELEMETRY {
                graph.channel_builder()
                    .with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10))
                    .with_type() //TODO: not sure why total is not changing here.

            } else {
                graph.channel_builder()
                    .with_compute_refresh_window_bucket_bits(0, 0)
            };

            //build channel for DiagramData type
            let (tx, rx) = base.with_labels(&["steady_state-telemetry"], true)
                .with_capacity(config::REAL_CHANNEL_LENGTH_TO_FEATURE)
                .build();

            let outgoing = Some(tx);

            let bldr = graph.actor_builder()
                .with_name(metrics_server::NAME);

            let bldr = if config::SHOW_TELEMETRY_ON_TELEMETRY {
                bldr.with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10))
                    .with_avg_mcpu()
                    .with_avg_work()
            } else {
                bldr
            };

            bldr.build_with_exec(move |context|
                telemetry::metrics_server::run(context
                                               , rx.clone()
                ));

            if config::TELEMETRY_SERVER  {
                let all_tel_rx = graph.all_telemetry_rx.clone(); //using Arc here

                let bldr = graph.actor_builder()
                    .with_name(metrics_collector::NAME);

                let bldr = if config::SHOW_TELEMETRY_ON_TELEMETRY {
                    bldr.with_compute_refresh_window_floor(Duration::from_secs(1),Duration::from_secs(10))
                        .with_avg_mcpu()
                        .with_avg_work()
                } else {
                    bldr
                };

                bldr.build_with_exec(move |context| {
                    let all_rx = all_tel_rx.clone();
                    telemetry::metrics_collector::run(context
                                                      , all_rx
                                                      , outgoing.clone()
                    )
                });
            }
        }
}

#[inline]
pub(crate) async fn try_send_all_local_telemetry<const RX_LEN: usize, const TX_LEN: usize>(this: &mut LocalMonitor<RX_LEN, TX_LEN>) {
    //do not relay faster than this to ensure we do not overload the channel
    //we will continue to roll up data if we do not send it
    let last_elapsed = this.last_telemetry_send.elapsed();
    if last_elapsed.as_micros() >= config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u128 {

        let is_in_graph: bool = this.is_in_graph();

        if let Some(ref mut actor_status) = this.telemetry_state {
            {
                let msg = actor_status.status_message();
                //NOTE: actor status MUST be sent every MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS
                //      until we reach half of channel capacity, then we will back off
                //      this fill level is used to "trigger" the collector as a timer
                //      channel take/send data can skip tx if it is all zeros but that
                //      can never be done here since this is critical for the collector



                if is_in_graph {
                    let clear_status = {
                        let mut lock_guard = actor_status.tx.lock().await;
                        let tx = lock_guard.deref_mut();

                        let capacity = tx.capacity();
                        let vacant_units = tx.vacant_units();

                        if vacant_units >= (capacity>>1) {
                            //ok we are not in danger of filling up
                        } else {
                            let scale = calculate_exponential_channel_backoff(capacity, vacant_units);
                            if last_elapsed.as_micros() < scale as u128 * config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u128 {

                                if let Ok(guard) = this.runtime_state.read() {
                                    let state = guard.deref();
                                    //if we are not shutting down we may need to report this error
                                    if !state.is_in_state(&[GraphLivelinessState::StopRequested
                                                                  ,GraphLivelinessState::Stopped
                                                                  ,GraphLivelinessState::StoppedUncleanly]) {
                                        if scale>30 {
                                            //TODO: this hard exit to be removed when we release version 1
                                            error!("{:?} EXIT hard delay on actor status: {} empty{} of{}",this.ident,scale,vacant_units,capacity);
                                            error!("assume metrics_collector has died and is not consuming messages");
                                            exit(-1);
                                        }
                                    }
                                }
                                //we have discovered that the consumer is not keeping up
                                //so we will make use of the exponential backoff.
                                return;
                            }
                        }
                        match tx.try_send(msg) {
                            Ok(_) => {
                                if let Some(ref mut send_tx) = this.telemetry_send_tx {
                                    if tx.local_index.lt(&MONITOR_NOT) {
                                        //only record those turned on (init)
                                        //this happy path where we already have our index
                                        send_tx.count[tx.local_index] += 1;
                                        //we know this has already happened once therefore we can check for slowness
                                        //between now the last time we sent telemetry
                                        if last_elapsed.as_millis() >= config::TELEMETRY_PRODUCTION_RATE_MS as u128 {
                                            let now = Instant::now();
                                            let dif = now.duration_since(actor_status.last_telemetry_error);
                                            if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                                warn!("{:?} consider shortening period of relay_stats_periodic or adding relay_stats_all() in your work loop,\n it is called too infrequently at {}ms which is larger than your frame rate of {}ms",this.ident, last_elapsed.as_millis(), config::TELEMETRY_PRODUCTION_RATE_MS);
                                                actor_status.last_telemetry_error = now;
                                            }
                                        }
                                    } else if tx.local_index.eq(&MONITOR_UNKNOWN) {
                                        tx.local_index = find_my_index(send_tx, tx.channel_meta_data.id);
                                        if tx.local_index.lt(&MONITOR_NOT) {
                                            send_tx.count[tx.local_index] += 1;
                                        }
                                    }
                                }
                                true
                            },
                            Err(a) => {
                                let now = Instant::now();
                                let dif = now.duration_since(actor_status.last_telemetry_error);
                                if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {

                                    if let Ok(guard) = this.runtime_state.read() {
                                        let state = guard.deref();
                                        //if we are not shutting down we may need to report this error
                                        if !state.is_in_state(&[GraphLivelinessState::StopRequested
                                            , GraphLivelinessState::Stopped
                                            , GraphLivelinessState::StoppedUncleanly]) {
                                            //Check metrics_consumer for slowness
                                            warn!("full telemetry state channel detected from {:?} value:{:?} full:{:?} capacity: {:?} "
                                             , this.ident, a, tx.is_full(), tx.tx.capacity());
                                            //store time to do this again later.
                                            actor_status.last_telemetry_error = now;
                                        }
                                    }


                                }
                                false
                            }
                        }
                    };
                    if clear_status {
                        actor_status.status_reset();
                    }
                } else {
                    actor_status.status_reset();
                }
            }
        }

        if let Some(ref mut send_tx) = this.telemetry_send_tx {
            // switch to a new vector and send the old one
            // we always clear the count so we can confirm this in testing

            if send_tx.count.iter().any(|x| !x.is_zero()) {

                //we only send the result if we have a context, ie a graph we are monitoring
                if is_in_graph {
                    let mut lock_guard = send_tx.tx.lock().await;
                    let tx = lock_guard.deref_mut();
                    match tx.try_send(send_tx.count) {
                        Ok(_) => {
                            send_tx.count.fill(0);
                            if tx.local_index.lt(&MONITOR_NOT) { //only record those turned on (init)
                                send_tx.count[tx.local_index] = 1;
                            } else if tx.local_index.eq(&MONITOR_UNKNOWN) {
                                    tx.local_index = find_my_index(send_tx, tx.channel_meta_data.id);
                                    if tx.local_index.lt(&MONITOR_NOT) {
                                        send_tx.count[tx.local_index] = 1;
                                    }
                            }
                        },
                        Err(a) => {
                            let now = Instant::now();
                            let dif = now.duration_since(send_tx.last_telemetry_error);
                            if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                //Check metrics_consumer for slowness
                                warn!("full telemetry tx channel detected from {:?} value:{:?} full:{:?} capacity: {:?} "
                                             , this.ident, a, tx.is_full(), tx.tx.capacity());
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

        if let Some(ref mut send_rx) = this.telemetry_send_rx {
            if send_rx.count.iter().any(|x| !x.is_zero()) {

                //we only send the result if we have a context, ie a graph we are monitoring
                if is_in_graph {
                    let mut lock_guard = send_rx.tx.lock().await;
                    let rx = lock_guard.deref_mut();
                    match rx.try_send(send_rx.count) {
                        Ok(_) => {
                            send_rx.count.fill(0);
                            if rx.local_index.lt(&MONITOR_NOT) { //only record those turned on (init)
                                send_rx.count[rx.local_index] = 1;
                            } else if rx.local_index.eq(&MONITOR_UNKNOWN) {
                                rx.local_index = find_my_index(send_rx, rx.channel_meta_data.id);
                                if rx.local_index.lt(&MONITOR_NOT) {
                                    send_rx.count[rx.local_index] = 1;
                                }
                            }

                        },
                        Err(a) => {
                            let now = Instant::now();
                            let dif = now.duration_since(send_rx.last_telemetry_error);
                            if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                //Check metrics_consumer for slowness
                                warn!("full telemetry rx channel detected from {:?} value:{:?} full:{:?} capacity: {:?} "
                                             , this.ident, a, rx.is_full(), rx.tx.capacity());

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
        this.last_telemetry_send = Instant::now();

    }
}

pub(crate) fn calculate_exponential_channel_backoff(capacity: usize, vacant_units: usize) -> u32 {
    // Number of bits required to represent the capacity
    let bits_count = (capacity as f64).log2().ceil() as u32;
    // Number of bits required in the u32 vacant count
    let bit_to_represent_vacant_count = 32-(vacant_units as u32).leading_zeros();
    // If vacant is large then our delay will be small by design
    // As vacant gets small the delay will grow exponentially
    (1 + bits_count - bit_to_represent_vacant_count).pow(3)
}


pub(crate) async fn send_all_local_telemetry_async<const RX_LEN: usize, const TX_LEN: usize>(this: &mut LocalMonitor<RX_LEN, TX_LEN>) {
        // send the last data before this struct is dropped.

        #[cfg(debug_assertions)]
        trace!("start last send of all local telemetry");

        if this.is_in_graph() {
            if let Some(ref mut actor_status) = this.telemetry_state {
                let msg = actor_status.status_message();
                    {
                        let mut lock_guard = actor_status.tx.lock().await;
                        let tx = lock_guard.deref_mut();
                        //TODO: prod/dev bool
                        let _ = tx.send_async(this.ident, msg,false).await;
                    }
                    actor_status.status_reset();

            }
            if let Some(ref mut send_tx) = this.telemetry_send_tx {
                if send_tx.count.iter().any(|x| !x.is_zero()) {
                    let mut lock_guard = send_tx.tx.lock().await;
                    let tx = lock_guard.deref_mut();
                    //TODO: prod/dev bool
                    let _ = tx.send_async(this.ident,send_tx.count,false).await;
                }
            }
            if let Some(ref mut send_rx) = this.telemetry_send_rx {
                if send_rx.count.iter().any(|x| !x.is_zero()) {
                    let mut lock_guard = send_rx.tx.lock().await;
                    let rx = lock_guard.deref_mut();
                    //TODO: prod/dev bool
                    let _ = rx.send_async(this.ident, send_rx.count,false).await;
                }
            }

            if let Some(ref mut actor_status) = this.telemetry_state {
                    let mut lock_guard = actor_status.tx.lock().await;
                    let tx = lock_guard.deref_mut();
                    tx.wait_empty().await;
            }
            if let Some(ref send_tx) = this.telemetry_send_tx {
                if send_tx.count.iter().any(|x| !x.is_zero()) {
                    let mut lock_guard = send_tx.tx.lock().await;
                    let tx = lock_guard.deref_mut();
                    tx.wait_empty().await;
                }
            }
            if let Some(ref send_rx) = this.telemetry_send_rx {
                if send_rx.count.iter().any(|x| !x.is_zero()) {
                    let mut lock_guard = send_rx.tx.lock().await;
                    let rx = lock_guard.deref_mut();
                    rx.wait_empty().await;
                }
            }

        }
    #[cfg(debug_assertions)]
    trace!("send of all local telemetry complete");

}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn test_compute_scale_up_delay() {

        let capacity = 128;

        for vacant_units in (0..capacity).rev() {
            let backoff = calculate_exponential_channel_backoff(capacity, vacant_units);
            //  println!("vacant:{} backoff:{}",vacant_units, backoff);
            match vacant_units {
                64..=127 => assert_eq!(backoff, 1), //the first half is always 1
                32..=63 =>  assert_eq!(backoff, 8),
                16..=31 => assert_eq!(backoff, 27), //then we rapidly scale up
                8..=15 =>  assert_eq!(backoff, 64),
                4..=7 =>  assert_eq!(backoff, 125),
                2..=3 =>  assert_eq!(backoff, 216),
                1 =>      assert_eq!(backoff, 343),
                0 =>      assert_eq!(backoff, 512),
                _=> {}
            }
        }

    }


}

