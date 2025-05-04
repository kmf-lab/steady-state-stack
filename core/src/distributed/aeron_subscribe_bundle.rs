use std::error::Error;
use std::ops::{Div, Mul};
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
use crate::{await_for_all, IntoSimRunner, SteadyCommander, SteadyState, SteadyStreamTxBundleTrait, StreamTx, StreamTxBundleTrait, TestEcho};
use crate::commander_context::SteadyContext;
use crate::core_tx::TxCore;
use crate::distributed::polling;
use crate::yield_now;
#[derive(Default)]
pub struct AeronSubscribeSteadyState {
    sub_reg_id: Vec<Option<i64>>,
}

const ROUND_ROBIN:Option<Duration> = None;

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

    let mut input_bytes: u32 = 0;
    let mut input_frags: u32 = 0;

    let now = Instant::now();

    let measured_vacant_items = tx_item.item_channel.shared_vacant_units() as u32;
    let measured_vacant_bytes = tx_item.payload_channel.shared_vacant_units() as u32;

    loop {
        let remaining_poll = if let Some(s) = tx_item.smallest_space() { s } else {
            tx_item.item_channel.capacity()
        };
       //error!("poll for as many as: {}", remaining_poll);
        if 0 >= sub.poll(&mut | buffer: &AtomicBuffer,
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
            input_frags += 1;
        }, remaining_poll as i32) {
            break; //we got no data so leave now we will try again later
        }
        yield_now().await; //do not remove in many cases this allows us more data in this pass
    }
    //trace!("poll the stream {} got {} ready msg empty {} ",sub.stream_id(),input_frags, tx_item.ready_msg_session.is_empty());

    if !tx_item.ready_msg_session.is_empty() {
        let (now_sent_messages, now_sent_bytes) = tx_item.fragment_flush_ready(cmd);
        let (stored_vacant_items, stored_vacant_bytes) = tx_item.get_stored_vacant_values();

        if stored_vacant_items > measured_vacant_items as i32 {
            let duration = now.duration_since(tx_item.last_output_instant);
            tx_item.store_output_data_rate(duration
                              , (stored_vacant_items - measured_vacant_items as i32) as u32
                              , (stored_vacant_bytes - measured_vacant_bytes as i32) as u32);

            tx_item.last_output_instant = now;
            let new_vacant_items:i32 = measured_vacant_items as i32 - now_sent_messages as i32;
            let new_vacant_bytes:i32 = measured_vacant_bytes as i32 - now_sent_bytes as i32;
            tx_item.set_stored_vacant_values(new_vacant_items, new_vacant_bytes);
        }
    }
    if input_frags > 0 {
        let duration = now.duration_since(tx_item.last_input_instant);
        tx_item.store_input_data_rate(duration, input_frags, input_bytes);
        tx_item.last_input_instant = now;
    }

    let (avg,std) = tx_item.guess_duration_between_arrivals();
    let (min,max) = tx_item.next_poll_bounds();

    let mut scheduler = polling::PollScheduler::new();
    scheduler.set_max_delay_ns(max.as_nanos() as u64);
    scheduler.set_min_delay_ns(min.as_nanos() as u64);
    scheduler.set_std_dev_ns(std.as_nanos() as u64);
    scheduler.set_expected_moment_ns(avg.as_nanos() as u64);
    let now_ns = now.duration_since(tx_item.last_input_instant).as_nanos() as u64;
    let ns = scheduler.compute_next_poll_time(now_ns);
    Duration::from_nanos(ns)
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

    let mut tx_guards = tx_bundle.lock().await;
    for f in 0..GIRTH {
        if let Some(id) = state.sub_reg_id[f] {
            let mut found = false;
            while cmd.is_running(&mut || tx_guards.mark_closed()) && !found {
                let sub = aeron.lock().await.find_subscription(id);
                match sub {
                    Err(e) => {
                        if e.to_string().contains("Awaiting")
                            || e.to_string().contains("not ready") {
                            Delay::new(Duration::from_millis(2)).await;
                            if cmd.is_liveliness_stop_requested() {
                                warn!("stop detected before finding publication");
                                subs[f] = Err("Shutdown requested while waiting".into());
                                found = true;
                            }
                        } else {
                            warn!("Error finding subscription: {:?}", e);
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
            let time_to_wait = earliest_time - now;
            if time_to_wait.as_secs() > 2 {
                error!("waiting {:?} Sec",time_to_wait.as_secs());
            }
            cmd.wait_periodic(time_to_wait).await;
            //TODO: we need some kind of check in the runtime to ensure cmd is in the stack for await.
        }
        now = Instant::now();
        {
            let tx_stream = &mut tx_guards[earliest_idx];
            //error!("AA earliest index selecte {:?} {:?} of {:?}", earliest_idx, tx_stream.shared_vacant_units(), tx_stream.capacity());

            let desired = if !tx_stream.shared_is_full() {
                if let Ok(sub) = &mut subs[earliest_idx] {
                    let delay = poll_aeron_subscription(tx_stream, sub, &mut cmd).await;
                    now + delay
                } else {
                    error!("Internal error, the subscription should be present");
                    //moving this out of the way to avoid checking again
                    now + Duration::from_secs(i32::MAX as u64)
                }
            } else {
                  //trace!("output full skipping {:?}", earliest_idx);
                  if let Some(f) = tx_stream.fastest_byte_processing_duration() {
                      now + f.mul((tx_stream.capacity() >> 1) as u32)
                  } else {
                      now + tx_stream.max_poll_latency
                  }
            };
            next_times[earliest_idx] = if let Some(d) = ROUND_ROBIN {now+d} else {desired};

        }
    }
    Ok(())
}