use std::collections::VecDeque;
use std::ops::{Deref, DerefMut, Sub};
use std::sync::Arc;
use std::time::{Duration, Instant};
use log::*;
use num_traits::Zero;
use ringbuf::traits::Observer;
use crate::{config, Graph, LocalMonitor, MONITOR_NOT, MONITOR_UNKNOWN, SteadyContext, telemetry};
use crate::channel_builder::{ChannelBuilder};
use crate::config::MAX_TELEMETRY_ERROR_RATE_SECONDS;
use crate::monitor::{ChannelMetaData, find_my_index, RxTel, SteadyTelemetryActorSend, SteadyTelemetryRx, SteadyTelemetrySend, SteadyTelemetryTake};
use crate::telemetry::metrics_collector::CollectorDetail;

pub(crate) fn construct_telemetry_channels<const RX_LEN: usize, const TX_LEN: usize>(that: &SteadyContext
                                                                                     , rx_meta_data: Vec<Arc<ChannelMetaData>>
                                                                                     , rx_inverse_local_idx: [usize; RX_LEN]
                                                                                     , tx_meta_data: Vec<Arc<ChannelMetaData>>
                                                                                     , tx_inverse_local_idx: [usize; TX_LEN]) -> (Option<SteadyTelemetrySend<{ RX_LEN }>>, Option<SteadyTelemetrySend<{ TX_LEN }>>, Option<SteadyTelemetryActorSend>) {
//NOTE: if this child telemetry is monitored so we will create the appropriate channels
    let start_now = Instant::now().sub(Duration::from_secs(1 + MAX_TELEMETRY_ERROR_RATE_SECONDS as u64));

    let channel_builder = ChannelBuilder::new(that.channel_count.clone())
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

    //need to hand off to the collector
    let mut shared_vec_guard = bastion::run!(that.all_telemetry_rx.lock());

    let shared_vec = shared_vec_guard.deref_mut();
    let index:Option<usize> = shared_vec.iter()
                            .enumerate()
                            .find(|(_,x)|x.monitor_id == that.ident.id && x.name == that.ident.name)
                            .map(|(idx,_)|idx);
    if let Some(idx) = index {
      //we add new SteadyTelemetryRx which waits for the old one to be consumed first
      shared_vec[idx].telemetry_take.push_back(Box::new(det));
    } else {
        let mut tt:VecDeque<Box<dyn RxTel>> = VecDeque::new();
        tt.push_back(Box::new(det));
        let details = CollectorDetail {
            name: that.ident.name,
            monitor_id: that.ident.id,
            temp_barrier: false,
            telemetry_take: tt,
        };
        shared_vec.push(details); //add new telemetry channels
    }


    let telemetry_actor =

        Some(SteadyTelemetryActorSend {
            tx: act_tuple.0,
            last_telemetry_error: start_now,
            await_ns_unit: 0,
            instant_start: Instant::now(),
            hot_profile: None,
            redundancy: 1,
            calls: [0; 6],
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
            //build channel for DiagramData type
            let (tx, rx) = graph.channel_builder()
                .with_compute_refresh_window_bucket_bits(0, 0)
                .with_labels(&["steady_state-telemetry"], true)
                .with_capacity(config::REAL_CHANNEL_LENGTH_TO_FEATURE)
                .build();

            let outgoing = Some(tx);

            let bldr = graph.actor_builder()
                .with_name("telemetry-polling");

            let bldr = if config::SHOW_TELEMETRY_ON_TELEMETRY {
                bldr.with_avg_mcpu().with_avg_work()
            } else {
                bldr
            };

            bldr.build_with_exec(move |context|
                telemetry::metrics_server::run(context
                                               , rx.clone()
                ));


            let senders_count: usize = {
                let guard = bastion::run!(graph.all_telemetry_rx.lock());
                let v = guard.deref();
                v.len()
            };

            //only spin up the metrics collector if we have a consumer
            //OR if some actors are sending data we need to consume
            if config::TELEMETRY_SERVER || senders_count > 0 {
                let all_tel_rx = graph.all_telemetry_rx.clone(); //using Arc here

                let bldr = graph.actor_builder()
                    .with_name("telemetry-collector");

                let bldr = if config::SHOW_TELEMETRY_ON_TELEMETRY {
                    bldr.with_avg_mcpu().with_avg_work()
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


pub(crate) async fn try_send_all_local_telemetry<const RX_LEN: usize, const TX_LEN: usize>(this: &mut LocalMonitor<RX_LEN, TX_LEN>) {
//only relay if we are withing the bounds of the telemetry channel limits.
    if this.last_telemetry_send.elapsed().as_micros() >= config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u128 {

        let is_in_graph: bool = this.is_in_graph();

        if let Some(ref mut actor_status) = this.telemetry_state {
            if let Some(msg) = actor_status.status_message() {
                if is_in_graph {
                    let clear_status = {
                        let mut lock_guard = actor_status.tx.lock().await;
                        let tx = lock_guard.deref_mut();
                        match tx.try_send(msg) {
                            Ok(_) => {
                                if let Some(ref mut send_tx) = this.telemetry_send_tx {
                                    if tx.local_index.lt(&MONITOR_NOT) { //only record those turned on (init)
                                        send_tx.count[tx.local_index] += 1;
                                    } else if tx.local_index.eq(&MONITOR_UNKNOWN) {
                                        tx.local_index = find_my_index(send_tx, tx.id);
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
                                    //Check metrics_consumer for slowness
                                    error!("relay all tx, full telemetry channel detected upon tx from telemetry: {:?} value:{:?} full:{:?} capacity: {:?} "
                                             , this.ident.name, a, tx.is_full(), tx.tx.capacity());
                                    //store time to do this again later.
                                    actor_status.last_telemetry_error = now;
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
                                    tx.local_index = find_my_index(send_tx, tx.id);
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
                                error!("relay all tx, full telemetry channel detected upon tx from telemetry: {} value:{:?} full:{} "
                                             , this.ident.name, a, tx.is_full());
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
                                rx.local_index = find_my_index(send_rx, rx.id);
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
                                error!("relay all rx, full telemetry channel detected upon rx from telemetry: {} value:{:?} full:{} "
                                             , this.ident.name, a, rx.is_full());
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


pub(crate) async fn send_all_local_telemetry_async<const RX_LEN: usize, const TX_LEN: usize>(this: &mut LocalMonitor<RX_LEN, TX_LEN>) {
        // send the last data before this struct is dropped.

        #[cfg(debug_assertions)]
        trace!("start last send of all local telemetry");

        if this.is_in_graph() {
            if let Some(ref mut actor_status) = this.telemetry_state {
                if let Some(msg) = actor_status.status_message() {
                    {
                        let mut lock_guard = actor_status.tx.lock().await;
                        let tx = lock_guard.deref_mut();
                        let _ = tx.send_async(msg).await;
                    }
                    actor_status.status_reset();
                }
            }
            if let Some(ref mut send_tx) = this.telemetry_send_tx {
                if send_tx.count.iter().any(|x| !x.is_zero()) {
                    let mut lock_guard = send_tx.tx.lock().await;
                    let tx = lock_guard.deref_mut();
                    let _ = tx.send_async(send_tx.count).await;
                }
            }
            if let Some(ref mut send_rx) = this.telemetry_send_rx {
                if send_rx.count.iter().any(|x| !x.is_zero()) {
                    let mut lock_guard = send_rx.tx.lock().await;
                    let rx = lock_guard.deref_mut();
                    let _ = rx.send_async(send_rx.count).await;
                }
            }


            if let Some(ref mut actor_status) = this.telemetry_state {
                if actor_status.status_message().is_some() {
                    let mut lock_guard = actor_status.tx.lock().await;
                    let tx = lock_guard.deref_mut();
                    tx.wait_empty().await;
                }
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

