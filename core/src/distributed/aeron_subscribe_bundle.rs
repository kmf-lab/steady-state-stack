use std::error::Error;
use std::ops::Div;
use std::sync::Arc;
use std::time::{Duration, Instant};
use futures_timer::Delay;
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::AtomicBuffer;
use aeron::concurrent::logbuffer::frame_descriptor;
use aeron::concurrent::logbuffer::header::Header;
use aeron::subscription::Subscription;
use async_std::sync::Mutex;
use log::{error, trace, warn};
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::distributed_stream::{SteadyStreamTxBundle, StreamSessionMessage};
use crate::{IntoSimRunner, SteadyCommander, SteadyState, SteadyStreamTxBundleTrait, StreamTx, StreamTxBundleTrait, TestEcho};
use crate::commander_context::SteadyContext;
use crate::core_tx::TxCore;

#[derive(Default)]
pub struct AeronSubscribeSteadyState {
    sub_reg_id: Vec<Option<i64>>,
}

pub async fn run<const GIRTH: usize>(
    context: SteadyContext,
    tx: SteadyStreamTxBundle<StreamSessionMessage, GIRTH>,
    aeron_connect: Channel,
    stream_id: i32,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let mut cmd = context.into_monitor([], tx.control_meta_data());
    if cmd.use_internal_behavior {
        while cmd.aeron_media_driver().is_none() {
            warn!("unable to find Aeron media driver, will try again in 15 sec");
            let mut tx = tx.lock().await;
            if cmd.is_running(&mut || tx.mark_closed()) {
                let _ = cmd.wait_periodic(Duration::from_secs(15)).await;
            } else {
                return Ok(());
            }
        }
        let aeron_media_driver = cmd.aeron_media_driver().expect("media driver");
        return internal_behavior(cmd, tx, aeron_connect, stream_id, aeron_media_driver, state).await;
    }
    let te: Vec<_> = tx.iter().map(|f| TestEcho(f.clone())).collect();
    let sims: Vec<_> = te.iter().map(|f| f as &dyn IntoSimRunner<_>).collect();
    cmd.simulated_behavior(sims).await
}

async fn poll_aeron_subscription<C: SteadyCommander>(
    tx_item: &mut StreamTx<StreamSessionMessage>,
    sub: &mut Subscription,
    cmd: &mut C,
) -> Duration {
    let remaining_poll = if let Some(s) = tx_item.smallest_space() { s } else {
        tx_item.item_channel.capacity()
    };
    let mut input_bytes: u32 = 0;

    let now = Instant::now();

    let measured_vacant_items = tx_item.item_channel.shared_vacant_units() as u32;
    let measured_vacant_bytes = tx_item.payload_channel.shared_vacant_units() as u32;

    let input_frags: u32 = sub.poll(&mut |buffer: &AtomicBuffer,
                                          offset: i32,
                                          length: i32,
                                          header: &Header| {
        let flags = header.flags();
        let is_begin: bool = 0 != (flags & frame_descriptor::BEGIN_FRAG);
        let is_end: bool = 0 != (flags & frame_descriptor::END_FRAG);
        tx_item.fragment_consume(
            header.session_id(),
            buffer.as_sub_slice(offset, length),
            is_begin,
            is_end,
            now,
        );
        input_bytes += length as u32;
    }, remaining_poll as i32) as u32;

    error!("poll the session {} got {}",sub.stream_id(),input_frags);

    if !tx_item.ready_msg_session.is_empty() {
        let (now_sent_messages, now_sent_bytes) = tx_item.fragment_flush_ready(cmd);
        let (stored_vacant_items, stored_vacant_bytes) = tx_item.get_stored_vacant_values();

        assert!(measured_vacant_items <= stored_vacant_items);
        if measured_vacant_items != stored_vacant_items {
            let duration = now.duration_since(tx_item.last_output_instant);
            tx_item.store_output_data_rate(duration
                              , stored_vacant_items - measured_vacant_items
                              , stored_vacant_bytes - measured_vacant_bytes);

            tx_item.last_output_instant = now;
            let new_vacant_items = measured_vacant_items - now_sent_messages;
            let new_vacant_bytes = measured_vacant_bytes - now_sent_bytes;
            tx_item.set_stored_vacant_values(new_vacant_items, new_vacant_bytes);
        }
    }
    if input_frags > 0 {
        let duration = now.duration_since(tx_item.last_input_instant);
        tx_item.store_input_data_rate(duration, input_frags, input_bytes);
        tx_item.last_input_instant = now;
    }

    let (a,b) = tx_item.guess_duration_between_arrivals();
    let minimum = tx_item.guess_duration_till_empty().div(2);
    warn!("poll details {:?} send: min{:?} arrivals: mean{:?} std{:?}",sub.stream_id(), minimum, a, b);
    //TODO: here we will use the new delay compute.

    Duration::from_millis(10)
}

async fn internal_behavior<const GIRTH: usize, C: SteadyCommander>(
    mut cmd: C,
    tx: SteadyStreamTxBundle<StreamSessionMessage, GIRTH>,
    aeron_channel: Channel,
    stream_id: i32,
    aeron: Arc<futures_util::lock::Mutex<Aeron>>,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let tx_bundle = tx;
    let mut state = state.lock(|| AeronSubscribeSteadyState::default()).await;
    let mut subs: [Result<Subscription, Box<dyn Error>>; GIRTH] = std::array::from_fn(|_| Err("Not Found".into()));

    while state.sub_reg_id.len() < GIRTH {
        state.sub_reg_id.push(None);
    }

    for f in 0..GIRTH {
        if state.sub_reg_id[f].is_none() {
            warn!("adding new sub {} {:?}", f as i32 + stream_id, aeron_channel.cstring());
            match aeron.lock().await.add_subscription(aeron_channel.cstring(), f as i32 + stream_id) {
                Ok(reg_id) => {
                    trace!("new subscription found: {}", reg_id);
                    state.sub_reg_id[f] = Some(reg_id);
                },
                Err(e) => {
                    warn!("Unable to add publication: {:?}", e);
                }
            };
        }
    }
    Delay::new(Duration::from_millis(2)).await;

    let mut tx_guards = tx_bundle.lock().await;
    for f in 0..GIRTH {
        if let Some(id) = state.sub_reg_id[f] {
            let mut found = false;
            while cmd.is_running(&mut || tx_guards.mark_closed()) && !found {
                let sub = aeron.lock().await.find_subscription(id);
                match sub {
                    Err(e) => {
                        if e.to_string().contains("Awaiting") || e.to_string().contains("not ready") {
                            Delay::new(Duration::from_millis(2)).await;
                            if cmd.is_liveliness_stop_requested() {
                                warn!("stop detected before finding publication");
                                subs[f] = Err("Shutdown requested while waiting".into());
                                found = true;
                            }
                        } else {
                            warn!("Error finding publication: {:?}", e);
                            subs[f] = Err(e.into());
                            found = true;
                        }
                    },
                    Ok(subscription) => {
                        match Arc::try_unwrap(subscription) {
                            Ok(mutex) => {
                                match mutex.into_inner() {
                                    Ok(subscription) => {
                                        subs[f] = Ok(subscription);
                                        found = true;
                                    },
                                    Err(_) => panic!("Failed to unwrap Mutex"),
                                }
                            },
                            Err(_) => panic!("Failed to unwrap Arc. Are there other references?"),
                        }
                    }
                }
            }
        } else {
            return Err("Check if Media Driver is running.".into());
        }
    }

    error!("running subscriber '{:?}' all subscriptions in place", cmd.identity());
    let mut next_times = [Instant::now(); GIRTH];
    let mut now = Instant::now();

    while cmd.is_running(&mut || tx_guards.mark_closed()) {
        let mut earliest_idx = 0;
        let mut earliest_time = next_times[0];
        for i in 1..GIRTH {
            if next_times[i] < earliest_time {
                earliest_time = next_times[i];
                earliest_idx = i;
            }
        }
        if earliest_time > now {
            async_std::task::sleep(earliest_time - now).await;
        }
        now = Instant::now();
        error!("earliest index selecte {:?}",earliest_idx);
        {
            let tx_stream = &mut tx_guards[earliest_idx];
            if !tx_stream.shared_is_full() {
                if let Ok(sub) = &mut subs[earliest_idx] {
                    let delay = poll_aeron_subscription(tx_stream, sub, &mut cmd).await;
                    next_times[earliest_idx] = now + delay;

                    error!("setting idx {:?} all next_times {:?}", earliest_idx, next_times);
                } else {
                    error!("Internal error, the subscription should be present");
                    //moving this out of the way to avoid checking again
                    next_times[earliest_idx] = now + Duration::from_secs(u32::MAX as u64);
                }
            } else {
                error!("output full skipping {:?}", earliest_idx);
                next_times[earliest_idx] = now + tx_stream.guess_duration_till_empty().div(2);
            }
        }
    }
    Ok(())
}