use std::collections::VecDeque;
use std::ops::{Deref, DerefMut, Sub};
use std::time::{Duration, Instant};
use bastion::{Bastion, run};
use bastion::prelude::SupervisionStrategy;
use log::*;
use num_traits::Zero;
use crate::{config, Graph, LocalMonitor, MONITOR_NOT, MONITOR_UNKNOWN, RxDef, SteadyContext, telemetry, TxDef};
use crate::channel_builder::{ChannelBuilder};
use crate::config::MAX_TELEMETRY_ERROR_RATE_SECONDS;
use crate::monitor::{find_my_index, RxTel, SteadyTelemetryActorSend, SteadyTelemetryRx, SteadyTelemetrySend, SteadyTelemetryTake};
use crate::telemetry::metrics_collector::CollectorDetail;

pub(crate) fn build_telemetry_channels<const RX_LEN: usize, const TX_LEN: usize>(that: &SteadyContext
                                                                      , rx_defs: &[&mut dyn RxDef; RX_LEN]
                                                                      , tx_defs: &[&mut dyn TxDef; TX_LEN]
) -> (Option<SteadyTelemetrySend< RX_LEN >>
      , Option<SteadyTelemetrySend< TX_LEN >>
      , Option<SteadyTelemetryActorSend>)
{

    let mut rx_meta_data = Vec::new();
    let mut rx_inverse_local_idx = [0; RX_LEN];
    rx_defs.iter()
        .enumerate()
        .for_each(|(c, rx)| {
            assert!(rx.meta_data().id < usize::MAX);
            rx_meta_data.push(rx.meta_data());
            rx_inverse_local_idx[c]=rx.meta_data().id;
        });

    let mut tx_meta_data = Vec::new();
    let mut tx_inverse_local_idx = [0; TX_LEN];
    tx_defs.iter()
        .enumerate()
        .for_each(|(c, tx)| {
            assert!(tx.meta_data().id < usize::MAX);
            tx_meta_data.push(tx.meta_data());
            tx_inverse_local_idx[c]=tx.meta_data().id;
        });

    //NOTE: if this child telemetry is monitored so we will create the appropriate channels
    let start_now = Instant::now().sub(Duration::from_secs(1+MAX_TELEMETRY_ERROR_RATE_SECONDS as u64));

    let channel_builder = ChannelBuilder::new(that.channel_count.clone())
        .with_labels(&["steady_state-telemetry"], false)
        .with_compute_window_bucket_bits(0)
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
    };



    //need to hand off to the collector
    run!(async{
                let mut shared_vec_guard = that.all_telemetry_rx.lock().await;
                let shared_vec = shared_vec_guard.deref_mut();
                let index:Option<usize> = shared_vec.iter()
                                        .enumerate()
                                        .find(|(_,x)|x.monitor_id == that.id && x.name == that.name)
                                        .map(|(idx,_)|idx);
                if let Some(idx) = index {
                  //we add new SteadyTelemetryRx which waits for the old one to be consumed first
                  shared_vec[idx].telemetry_take.push_back(Box::new(det));
                } else {
                    let mut tt:VecDeque<Box<dyn RxTel>> = VecDeque::new();
                    tt.push_back(Box::new(det));
                    let details = CollectorDetail {
                        name: that.name,
                        monitor_id: that.id,
                        temp_barrier: false,
                        telemetry_take: tt,
                    };
                    shared_vec.push(details); //add new telemetry channels
                }
            });

    let telemetry_actor =
            Some(SteadyTelemetryActorSend {
                tx: act_tuple.0,
                last_telemetry_error: start_now,
                await_ns_unit: 0,
                redundancy: 1,
                instant_start: Instant::now(),
                single_read_calls: 0,
                batch_read_calls: 0,
                single_write_calls: 0,
                batch_write_calls: 0,
                other_calls: 0,
                wait_calls: 0,
                count_restarts: that.count_restarts.clone(),
                bool_stop: false,
            });

    let telemetry_send_rx = rx_tuple.0;
    let telemetry_send_tx = tx_tuple.0;
    (telemetry_send_rx, telemetry_send_tx, telemetry_actor)
}



pub(crate) fn build_optional_telemetry_graph( graph: & mut Graph)
{
//The Troupe is restarted together if one telemetry fails

    let _ = Bastion::supervisor(|supervisor| {
        //NOTE: I think I want something more clean but this will have to do for now.
        let supervisor = supervisor.with_strategy(SupervisionStrategy::OneForOne);

        let mut outgoing = None;

        let supervisor = if config::TELEMETRY_SERVER {
            //build channel for DiagramData type
            let (tx, rx) = graph.channel_builder()
                .with_compute_window_bucket_bits(0)
                .with_labels(&["steady_state-telemetry"], true)
                .with_capacity(config::REAL_CHANNEL_LENGTH_TO_FEATURE)
                .build();

            outgoing = Some(tx);
            supervisor.children(|children| {
                graph.actor_builder("telemetry-polling")
                    .build(children,move |monitor|
                                       telemetry::metrics_server::run(monitor
                                                                      , rx.clone()
                                       )
                )
            })
        } else {
            supervisor
        };


        let senders_count: usize = {
            let guard = run!(graph.all_telemetry_rx.lock());
            let v = guard.deref();
            v.len()
        };

        //only spin up the metrics collector if we have a consumer
        //OR if some actors are sending data we need to consume
        if config::TELEMETRY_SERVER || senders_count > 0 {
            supervisor.children(|children| {
                //we create this child last so we can clone the rx_vec
                //and capture all the telemetry actors as well
                let all_tel_rx = graph.all_telemetry_rx.clone(); //using Arc here

                graph.actor_builder("telemetry-collector")
                    .build(children,move |monitor| {
                        let all_rx = all_tel_rx.clone();
                        telemetry::metrics_collector::run(monitor
                                                          , all_rx
                                                          , outgoing.clone()
                        )
                    }
                )
            }
            )
        } else {
            supervisor
        }
    }).expect("Telemetry supervisor creation error.");
}


pub(crate) async fn try_send_all_local_telemetry<const RX_LEN: usize, const TX_LEN: usize>(this: &mut LocalMonitor<RX_LEN, TX_LEN>) {
//only relay if we are withing the bounds of the telemetry channel limits.
    if this.last_instant.elapsed().as_micros() >= config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u128 {

        let is_in_bastion: bool = { this.ctx().is_some() };

        if let Some(ref mut actor_status) = this.telemetry_state {
            if let Some(msg) = actor_status.status_message() {
                if is_in_bastion {
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
                                    error!("relay all tx, full telemetry channel detected upon tx from telemetry: {:?} value:{:?} full:{:?} "
                                             , this.name, a, tx.is_full());
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
                if is_in_bastion {
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
                                             , this.name, a, tx.is_full());
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
                if is_in_bastion {
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
                                             , this.name, a, rx.is_full());
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
        this.last_instant = Instant::now();

    }
}


pub(crate) async fn send_all_local_telemetry_async<const RX_LEN: usize, const TX_LEN: usize>(this: &mut LocalMonitor<RX_LEN, TX_LEN>) {
        // send the last data before this struct is dropped.

        #[cfg(debug_assertions)]
        trace!("start last send of all local telemetry");

        if this.ctx().is_some() {
            if let Some(ref mut actor_status) = this.telemetry_state {
                if let Some(msg) = actor_status.status_message() {
                    let mut lock_guard = actor_status.tx.lock().await;
                    let tx = lock_guard.deref_mut();
                    let _ = tx.send_async(msg).await;
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

