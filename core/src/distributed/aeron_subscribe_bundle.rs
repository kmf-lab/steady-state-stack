use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
use futures_timer::Delay;
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::AtomicBuffer;
use aeron::concurrent::logbuffer::frame_descriptor;
use aeron::concurrent::logbuffer::header::Header;
use aeron::subscription::Subscription;
use log::{error, warn};
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::distributed_stream::{SteadyStreamTxBundle, StreamSessionMessage};
use crate::{IntoSimRunner, SteadyCommander, SteadyState, SteadyStreamTxBundleTrait, StreamTx, StreamTxBundleTrait};
use crate::commander_context::SteadyContext;
use crate::core_tx::TxCore;
use crate::distributed::polling;
use crate::yield_now;
#[derive(Default)]
pub struct AeronSubscribeSteadyState {
    sub_reg_id: Vec<Option<i64>>,
}

const ROUND_ROBIN:Option<Duration> = None;// Some(Duration::from_millis(5)); //TODO: hack for testing

pub async fn run<const GIRTH: usize>(
    context: SteadyContext,
    tx: SteadyStreamTxBundle<StreamSessionMessage, GIRTH>,
    aeron_connect: Channel,
    stream_id: i32,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let mut cmd = context.into_monitor([], tx.control_meta_data());
    if cmd.use_internal_behavior {
        error!("failure, we are using live aeron and should be using simulation.");
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
        internal_behavior(cmd, tx, aeron_connect, stream_id, aeron_media_driver, state).await
    } else {
        error!("success we are using simulated beviror for the aeron subscribe bundles");
        let te: Vec<_> = tx.iter().map(|f| f.clone()).collect();
        let sims: Vec<_> = te.iter().map(|f| f as &dyn IntoSimRunner<_>).collect();
        cmd.simulated_behavior(sims).await
    }
}

async fn poll_aeron_subscription<C: SteadyCommander>(
    tx_item: &mut StreamTx<StreamSessionMessage>,
    sub: &mut Subscription,
    cmd: &mut C

) -> Duration {
    if sub.channel_status() != aeron::concurrent::status::status_indicator_reader::CHANNEL_ENDPOINT_ACTIVE {
        error!("Subscription {} not active, status: {}", sub.stream_id(), sub.channel_status());
        return tx_item.max_poll_latency;
    }

    let mut input_bytes: u32 = 0;
    let mut input_frags: u32 = 0;
    let now = Instant::now();

    // Poll the subscription
    loop {
        let remaining_poll = if let Some(s) = tx_item.smallest_space() { s } else {
            tx_item.item_channel.capacity()
        };
        if 0 >= sub.poll(&mut |buffer: &AtomicBuffer, offset: i32, length: i32, header: &Header| {
            let flags = header.flags();
            let is_begin = 0 != (flags & frame_descriptor::BEGIN_FRAG);
            let is_end = 0 != (flags & frame_descriptor::END_FRAG);
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

            
            
            break; // No data received, exit loop
        }
        yield_now().await; // Allow more data in this pass
    }

    // Flush ready messages and update state
    if !tx_item.ready_msg_session.is_empty() {
        let (now_sent_messages, now_sent_bytes) = tx_item.fragment_flush_ready(cmd);

        // Get current vacant units after flushing
        let current_vacant_items = tx_item.item_channel.shared_vacant_units() as i32;
        let current_vacant_bytes = tx_item.payload_channel.shared_vacant_units() as i32;

        // Store output data rate using actual sent values
        let duration = now.duration_since(tx_item.last_output_instant);
        tx_item.store_output_data_rate(duration, now_sent_messages, now_sent_bytes);
        tx_item.last_output_instant = now;

        // Update stored vacant values with current state
        tx_item.set_stored_vacant_values(current_vacant_items, current_vacant_bytes);
    }

    // Store input data rate if fragments were received
    if input_frags > 0 {
        let duration = now.duration_since(tx_item.last_input_instant);
        tx_item.store_input_data_rate(duration, input_frags, input_bytes);
        tx_item.last_input_instant = now;
    }

    // Schedule next poll
    let (avg, std) = tx_item.guess_duration_between_arrivals();
    let (min, max) = tx_item.next_poll_bounds();

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
            let mut meda_driver = aeron.lock();
            let connection_string = aeron_channel.cstring();
            let stream_id = f as i32 + stream_id;

            match meda_driver.await.add_subscription(connection_string, stream_id) {
                Ok(reg_id) => {
                    state.sub_reg_id[f] = Some(reg_id);
                },
                Err(e) => {
                    warn!("Unable to register subscription: {:?}", e);
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
                            warn!("Idx: {} Error finding subscription: {:?}, trying registration process again", f, e);
                            subs[f] = Err(e.into());
                            let mut meda_driver = aeron.lock();
                            let connection_string = aeron_channel.cstring();
                            let stream_id = f as i32 + stream_id;
                            match meda_driver.await.add_subscription(connection_string, stream_id) {
                                Ok(reg_id) => {
                                    state.sub_reg_id[f] = Some(reg_id);
                                },
                                Err(e) => {
                                    warn!("Unable to register subscription: {:?}", e);
                                }
                            };
                        }
                    },
                    Ok(subscription) => {
                        match Arc::try_unwrap(subscription) {
                            Ok(mutex) => {
                                match mutex.into_inner() {
                                    Ok(subscription) => {
                                        //trace!("new sub {:?} status: {:?} connected: {:?}",subscription.stream_id(), subscription.channel_status(), subscription.is_connected());
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

    while cmd.is_running(&mut || tx_guards.mark_closed() ) {
        // if 0 == (loop_count % 10000) {
        //     log_count_down = 20;
        //     error!("---------------------------------------------------------------")
        // }
        // loop_count += 1;
        // let log_this = log_count_down > 0;
        // if log_this {
        //     log_count_down -= 1;
        // }

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
            cmd.wait_periodic(time_to_wait).await;    //TODO: we need some kind of check in the runtime to ensure cmd is in the stack for await.
        } 
        now = Instant::now();
        {
            let tx_stream = &mut tx_guards[earliest_idx];
           // error!("AA earliest index selecte {:?} {:?} of {:?}", earliest_idx, tx_stream.shared_vacant_units(), tx_stream.capacity());

            let dynamic = match &mut subs[earliest_idx] {
                        Ok(subscription) => {
                                poll_aeron_subscription(tx_stream, subscription, &mut cmd).await
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