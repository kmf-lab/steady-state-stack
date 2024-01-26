use std::ops::{Deref, DerefMut};
use std::time::Instant;
use bastion::{Bastion, run};
use bastion::prelude::SupervisionStrategy;
use log::error;
use num_traits::Zero;
use crate::steady_state::channel::ChannelBuilder;
use crate::steady_state::{config, graph, LocalMonitor, MONITOR_NOT, MONITOR_UNKNOWN, RxDef, SteadyContext, telemetry, TxDef};
use crate::steady_state::config::MAX_TELEMETRY_ERROR_RATE_SECONDS;
use crate::steady_state::Graph;
use crate::steady_state::monitor::*;
use crate::steady_state::telemetry::metrics_collector::CollectorDetail;


pub(crate) fn build_telemetry_channels<const RX_LEN: usize, const TX_LEN: usize>(that: &SteadyContext
                                                                      , rx_defs: &[&mut dyn RxDef; RX_LEN]
                                                                      , tx_defs: &[&mut dyn TxDef; TX_LEN]
) -> (Option<SteadyTelemetrySend<{ RX_LEN }>>, Option<SteadyTelemetrySend<{ TX_LEN }>>) {

    let mut rx_batch_limit = [0; RX_LEN];
    let mut rx_meta_data = Vec::new();
    let mut rx_inverse_local_idx = [0; RX_LEN];
    rx_defs.iter()
        .enumerate()
        .for_each(|(c, rx)| {
            rx_batch_limit[c] = rx.batch_limit();
            rx_meta_data.push(rx.meta_data());
            rx_inverse_local_idx[c]=rx.meta_data().id;
        });

    let mut tx_batch_limit = [0; TX_LEN];
    let mut tx_meta_data = Vec::new();
    let mut tx_inverse_local_idx = [0; TX_LEN];
    tx_defs.iter()
        .enumerate()
        .for_each(|(c, tx)| {
            tx_batch_limit[c] = tx.batch_limit();
            tx_meta_data.push(tx.meta_data());
            tx_inverse_local_idx[c]=tx.meta_data().id;
        });

    //NOTE: if this child telemetry is monitored so we will create the appropriate channels

    let rx_tuple: (Option<SteadyTelemetrySend<RX_LEN>>, Option<SteadyTelemetryTake<RX_LEN>>)
        = if 0usize == RX_LEN {
        (None, None)
    } else {
        let (telemetry_send_rx, mut telemetry_take_rx) =
            ChannelBuilder::new(that.channel_count.clone(), config::REAL_CHANNEL_LENGTH_TO_COLLECTOR)
                .with_labels(&["steady_state-telemetry"], false)
                .build();

        (Some(SteadyTelemetrySend::new(telemetry_send_rx, [0; RX_LEN], rx_batch_limit,that.name,rx_inverse_local_idx), )
         , Some(SteadyTelemetryTake { rx: telemetry_take_rx, details: rx_meta_data }))
    };

    let tx_tuple: (Option<SteadyTelemetrySend<TX_LEN>>, Option<SteadyTelemetryTake<TX_LEN>>)
        = if 0usize == TX_LEN {
        (None, None)
    } else {
        let (telemetry_send_tx, telemetry_take_tx) =
            ChannelBuilder::new(that.channel_count.clone(), config::REAL_CHANNEL_LENGTH_TO_COLLECTOR)
                .with_labels(&["steady_state-telemetry"], false)
                .build();

        (Some(SteadyTelemetrySend::new(telemetry_send_tx, [0; TX_LEN], tx_batch_limit,that.name,tx_inverse_local_idx), )
         , Some(SteadyTelemetryTake { rx: telemetry_take_tx, details: tx_meta_data }))
    };


    let details = CollectorDetail {
        name: that.name
        ,
        monitor_id: that.id
        ,
        telemetry_take: Box::new(SteadyTelemetryRx {
            send: tx_tuple.1,
            take: rx_tuple.1,
        })
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
                    shared_vec[idx] = details; //replace old telemetry channels
                } else {
                    shared_vec.push(details); //add new telemetry channels
                }
            });

    let telemetry_send_rx = rx_tuple.0;
    let telemetry_send_tx = tx_tuple.0;
    (telemetry_send_rx, telemetry_send_tx)
}



pub(crate) fn build_optional_telemetry_graph(graph: & mut Graph,) {
//The Troupe is restarted together if one telemetry fails
    let _ = Bastion::supervisor(|supervisor| {
        let supervisor = supervisor.with_strategy(SupervisionStrategy::OneForAll);

        let mut outgoing = None;

        let supervisor = if config::TELEMETRY_SERVER {
            //build channel for DiagramData type
            let (tx, rx) = graph.channel_builder(config::REAL_CHANNEL_LENGTH_TO_FEATURE)
                .with_labels(&["steady_state-telemetry"], true)
                .build();

            outgoing = Some(tx);
            supervisor.children(|children| {
                graph.add_to_graph("telemetry-polling"
                                   , children.with_redundancy(0)
                                   , move |monitor|
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

                graph::configure_for_graph(graph, "telemetry-collector"
                                           , children.with_redundancy(0)
                                           , move |monitor| {
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


pub(crate) async fn send_all_local_telemetry<const RX_LEN: usize, const TX_LEN: usize>(this: &mut LocalMonitor<RX_LEN, TX_LEN>) {
//only relay if we are withing the bounds of the telemetry channel limits.
    if this.last_instant.elapsed().as_micros() >= config::MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS as u128 {
        let is_in_bastion: bool = { this.ctx().is_some() };

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
                            } else {
                                if tx.local_index.eq(&MONITOR_UNKNOWN) {
                                    tx.local_index = find_my_index(send_tx, tx.id);
                                    if tx.local_index.lt(&MONITOR_NOT) {
                                        send_tx.count[tx.local_index] = 1;
                                    }
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
                            } else {
                                if rx.local_index.eq(&MONITOR_UNKNOWN) {
                                    rx.local_index = find_my_index(send_rx, rx.id);
                                    if rx.local_index.lt(&MONITOR_NOT) {
                                        send_rx.count[rx.local_index] = 1;
                                    }
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

