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

const ROUND_ROBIN:Option<Duration> = Some(Duration::from_millis(5)); //TODO: hack for testing

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
    log: bool
) -> Duration {

    if sub.channel_status() != aeron::concurrent::status::status_indicator_reader::CHANNEL_ENDPOINT_ACTIVE {
        error!("Subscription {} not active, status: {}", sub.stream_id(), sub.channel_status());
        return tx_item.max_poll_latency;
    }

    let mut input_bytes: u32 = 0;
    let mut input_frags: u32 = 0;

    let now = Instant::now();

    let measured_vacant_items = tx_item.item_channel.shared_vacant_units() as u32;
    let measured_vacant_bytes = tx_item.payload_channel.shared_vacant_units() as u32;

    loop {
        let remaining_poll = if let Some(s) = tx_item.smallest_space() { s } else {
            tx_item.item_channel.capacity()
        };
        //error!("sub: {} poll for as many as: {} status: {:?}", sub.stream_id(), remaining_poll, sub.channel_status());
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
            if  log {
                error!("on this poll pass we got {} {}",input_frags, input_bytes);
            }
            break; //we got no data so leave now we will try again later
        }
        yield_now().await; //do not remove in many cases this allows us more data in this pass
    }

    //error!("poll the stream {} got {} ready msg empty {} status {}",sub.stream_id(),input_frags, tx_item.ready_msg_session.is_empty(),sub.channel_status());

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
    let (min,max) = tx_item.next_poll_bounds();//TODO: check these limits?

    let mut scheduler = polling::PollScheduler::new();
    scheduler.set_max_delay_ns(max.as_nanos() as u64);
    scheduler.set_min_delay_ns(min.as_nanos() as u64);
    scheduler.set_std_dev_ns(std.as_nanos() as u64);
    scheduler.set_expected_moment_ns(avg.as_nanos() as u64);
    let waited_ns = now.duration_since(tx_item.last_input_instant).as_nanos() as u64;
    let ns = scheduler.compute_next_delay_ns(waited_ns);
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
                            Delay::new(Duration::from_millis(7)).await;
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
                                        error!("new sub {:?} status: {:?} connected: {:?}",subscription.stream_id(), subscription.channel_status(), subscription.is_connected());
                                        if subscription.is_connected() {
                                            subs[f] = Ok(subscription);
                                            found = true;                                            
                                        } else {
                                            Delay::new(Duration::from_millis(7)).await;
                                        }                                 
                                       
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
    let mut now = Instant::now();
    let mut next_times = [now; GIRTH];

    let mut log_count_down = 0;
    let mut loop_count = 0;
    while cmd.is_running(&mut || tx_guards.mark_closed()) {
        if 0 == (loop_count % 1000) {
            log_count_down = 10;
            error!("---------------------------------------------------------------")
        }
        loop_count += 1;
        let log_this = log_count_down > 0;
        if log_this {
            log_count_down -= 1;
        }

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
            if log_this || time_to_wait.as_secs() > 2 { //this should not be happening
                error!("idx {} waiting {:?} Sec",earliest_idx,time_to_wait.as_secs());
            }
            cmd.wait_periodic(time_to_wait).await;
            //TODO: we need some kind of check in the runtime to ensure cmd is in the stack for await.
        }  else {
            if log_this {
                error!("idx {} not waiting ",earliest_idx);
            }
        }


        now = Instant::now();
        {
            let tx_stream = &mut tx_guards[earliest_idx];
           // error!("AA earliest index selecte {:?} {:?} of {:?}", earliest_idx, tx_stream.shared_vacant_units(), tx_stream.capacity());

            let dynamic = match &mut subs[earliest_idx] {
                        Ok(subscription) => {
                                if log_this {
                                  //  [2025-05-08 23:30:23.085579 -05:00] T[async-std/runtime] ERROR [C:\Users\Getac\git\steady-state-stack\core\src\distributed\aeron_subscribe_bundle.rs:264] idx 0 poll_aeron_subscription 40 vacant (6400, 6400000) 6400 connected false closed false status 1
                                   error!("idx {:?} poll_aeron_subscription {:?} vacant {:?} {:?} connected {:?} closed {:?} status {:?}"
                                        , earliest_idx, subscription.stream_id()
                                        , tx_stream.get_stored_vacant_values(), tx_stream.shared_vacant_units()
                                        , subscription.is_connected(), subscription.is_closed(), subscription.channel_status()
                                    );
                                }
                                poll_aeron_subscription(tx_stream, subscription, &mut cmd, log_this).await
                        }
                        Err(e) => {error!("Internal error, the subscription should be present: {:?}",e);
                            //moving this out of the way to avoid checking again
                            Duration::from_secs(i32::MAX as u64)
                        }
                    };

            next_times[earliest_idx] = now + if let Some(fixed) = ROUND_ROBIN { fixed } else { dynamic };

        }
    }
    Ok(())
}