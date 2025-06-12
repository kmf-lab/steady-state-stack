use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
use futures_timer::Delay;
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::AtomicBuffer;
use aeron::concurrent::logbuffer::frame_descriptor;
use aeron::concurrent::logbuffer::header::Header;
use aeron::subscription::Subscription;
use log::{error, trace, warn};
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::distributed_stream::{SteadyStreamTxBundle, StreamIngress};
use crate::{SteadyActor, SteadyState, SteadyStreamTxBundleTrait, StreamTx, StreamTxBundleTrait};
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::core_tx::TxCore;
use crate::distributed::polling;
use crate::simulate_edge::IntoSimRunner;
use crate::yield_now;
#[derive(Default)]
pub struct AeronSubscribeSteadyState {
    sub_reg_id: Vec<Option<i64>>,
}

//TODO: time schedule has large waits we need to track down
const ROUND_ROBIN:Option<Duration> = Some(Duration::from_millis(1)); //TODO: hack for testing
//TODO: if publish is running some how subscribe needs to try again if too soon to get subcribe?

pub async fn run<const GIRTH: usize>(
    context: SteadyActorShadow,
    tx: SteadyStreamTxBundle<StreamIngress, GIRTH>,
    aeron_connect: Channel,
    stream_id: i32,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let mut actor = context.into_spotlight([], tx.payload_meta_data()); // TODO: must ensusre the other end had a payload selection!!!!!
    if actor.use_internal_behavior {
        while actor.aeron_media_driver().is_none() {
            warn!("unable to find Aeron media driver, will try again in 15 sec");
            let mut tx = tx.lock().await;
            if actor.is_running(&mut || tx.mark_closed()) {
                let _ = actor.wait_periodic(Duration::from_secs(15)).await;
            } else {
                return Ok(());
            }
        }
        internal_behavior(actor, tx, aeron_connect, stream_id, state).await
    } else {
        let te: Vec<_> = tx.iter().map(|f| f.clone()).collect();
        let sims: Vec<_> = te.iter().map(|f| f as &dyn IntoSimRunner<_>).collect();
        actor.simulated_behavior(sims).await
    }
}

async fn poll_aeron_subscription<C: SteadyActor>(
    tx_item: &mut StreamTx<StreamIngress>,
    sub: &mut Subscription,
    actor: &mut C,
    now: Instant

) -> Duration {
    //trace!("polling subscription {}", sub.stream_id());


    //read until count is zero or we have a pass without data.
    let mut count_down = 1;
    loop {
        let mut input_bytes: u32 = 0;
        let mut input_frags: u32 = 0;

        // Poll the subscription until no data or defrag is full
        loop {
            let remaining_poll = if let Some(s) = tx_item.smallest_space() { s } else {
                tx_item.control_channel.capacity()
            };
            if remaining_poll == 0 {
                if tx_item.shared_vacant_units()>0 {
                    error!("No space left in the buffer, tx room {:?} smallest {:?}", tx_item.shared_vacant_units(), tx_item.smallest_space());
                }
                break;
            }
            // warn!("sub.poll remaining_poll: {}", remaining_poll);
            let got_count = sub.poll(&mut |buffer: &AtomicBuffer, offset: i32, length: i32, header: &Header| {
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
            }, remaining_poll as i32);
            //  warn!("polling max of {} resulted in {} for sub {:?}", remaining_poll, got_count, sub.stream_id());

            if got_count <= 0 || got_count == (remaining_poll as i32) {
                break; // No data received, or we have data to pass on so exit loop
            }
            yield_now().await; // Allow more data in this pass
        }


        // Flush ready messages and update state
        if !tx_item.ready_msg_session.is_empty() {
            //trace!("flushing ready messages");
            let (now_sent_messages, now_sent_bytes) = tx_item.fragment_flush_ready(actor);

            // Get current vacant units after flushing
            let current_vacant_items = tx_item.control_channel.shared_vacant_units() as i32;
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
        } else {
            break;
        }
        count_down -= 1;
        if 0 == count_down {
            break;
        }
    }



    /////////////////////////////////////////////////
    // Schedule next poll
    /////////////////////////////////////////////////
    let (avg, std) = tx_item.guess_duration_between_arrivals();
    let (min, max) = tx_item.next_poll_bounds();

    let mut scheduler = polling::PollScheduler::new(); //too expensive??
    scheduler.set_max_delay_ns(max.as_nanos() as u64);
    scheduler.set_min_delay_ns(min.as_nanos() as u64);
    scheduler.set_std_dev_ns(std.as_nanos() as u64);
    scheduler.set_expected_moment_ns(avg.as_nanos() as u64);
    let waited_ns = now.duration_since(tx_item.last_input_instant).as_nanos() as u64;
    let ns = scheduler.compute_next_delay_ns(waited_ns);
    trace!("end of poll method {} ",ns);
    Duration::from_nanos(ns)
}

async fn internal_behavior<const GIRTH: usize, C: SteadyActor>(
    mut actor: C,
    tx: SteadyStreamTxBundle<StreamIngress, GIRTH>,
    aeron_channel: Channel,
    stream_id: i32,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let tx_bundle = tx;
    let mut state = state.lock(|| AeronSubscribeSteadyState::default()).await;
    let mut subs: [Result<Subscription, Box<dyn Error>>; GIRTH] = std::array::from_fn(|_| Err("Not Found".into()));

    while state.sub_reg_id.len() < GIRTH {
        state.sub_reg_id.push(None);
    }


    let aeron = actor.aeron_media_driver().expect("media driver");

    for f in 0..GIRTH {
        if state.sub_reg_id[f].is_none() {
            let connection_string = aeron_channel.cstring();
            let stream_id = f as i32 + stream_id;

            match aeron.lock().await.add_subscription(connection_string, stream_id) {
                Ok(reg_id) => {

                    error!("got this id {} for {}",reg_id,f);
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
            while actor.is_running(&mut || tx_guards.mark_closed()) && !found {
                    error!("looking for subscription {}",id);
                    let sub = {aeron.lock().await.find_subscription(id)};
                    match sub {
                        Err(e) => {
                            if actor.is_liveliness_stop_requested() {
                                trace!("stop detected before finding publication");
                                subs[f] = Err("Shutdown requested while waiting".into());
                                found = true; //needed to exit now
                            }
                            error!("error {:?} while looking for subscription {}",e,id);
                            actor.wait(Duration::from_millis(13)).await; //TODO: wait and wit periodic??
                            actor.relay_stats();
                        },
                        Ok(subscription) => {
                            error!("found subscription {}",id);
                            match Arc::try_unwrap(subscription) {
                                Ok(mutex) => { //we do NOT check is connected now because we only want to collect all the subscriptions.
                                    match mutex.into_inner() {
                                        Ok(subscription) => {
                                            //trace!("new sub {:?} status: {:?} connected: {:?}",subscription.stream_id(), subscription.channel_status(), subscription.is_connected());
                                            // let mut timeout = 100;
                                            // while !subscription.is_connected() && timeout>0 {
                                            //     Delay::new(Duration::from_millis(40)).await;
                                            //     timeout -= 1;
                                            // }
                                            //
                                            // if timeout>0 {
                                                subs[f] = Ok(subscription);
                                                found = true;
                                            // } else {
                                            //     drop(subscription);
                                            //      error!("unable to find open conneciton, remove and add subcription to try again");
                                            //     let connection_string = aeron_channel.cstring();
                                            //     let stream_id = f as i32 + stream_id;
                                            //
                                            //     match aeron.lock().await.add_subscription(connection_string, stream_id) {
                                            //         Ok(reg_id) => {
                                            //             error!("now got this id {} for {}",reg_id,f);
                                            //             state.sub_reg_id[f] = Some(reg_id);
                                            //         },
                                            //         Err(e) => {
                                            //             warn!("Unable to register subscription: {:?}", e);
                                            //         }
                                            //     };
                                            //     error!("unable to find open conneciton, remove and add subcription to try again");
                                            // }
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

    error!("running subscriber '{:?}' all subscriptions in place", actor.identity());
    let mut assume_connected = [false; GIRTH];

    for i in 0..GIRTH {
        match &subs[i] {
            Ok(subscription) => {
                let ref_images = subscription.images();
                assume_connected[i] = subscription.is_connected();
                warn!("{:?} connected: {:?} statuss: {:?} images: {:?}",i,assume_connected[i], subscription.channel_status(), ref_images.len());

            },
            Err(e) => {warn!("{:?} {:?}",i,e);}
        }

    }


    let mut now = Instant::now();
    let mut next_times = [now; GIRTH];
    let mut iteration = 0;

    while actor.is_running(&mut || tx_guards.mark_closed() ) {
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
            let tx_stream = &mut tx_guards[earliest_idx];
            if time_to_wait > tx_stream.max_poll_latency {
                trace!("time to wait is outside of the expected max {:?}",time_to_wait);
            }
            if time_to_wait > Duration::from_millis(5) {
                error!("big wait {:?} for {:?}",time_to_wait,earliest_idx);
            }
            // TODO: testing skip of wait
            actor.wait_periodic(time_to_wait).await;    //TODO: we need some kind of check in the runtime to ensure actor is in the stack for await.
        }
        now = Instant::now();
        {
            let tx_stream = &mut tx_guards[earliest_idx];
            if 0 == iteration &  ((1<<12)-1) {
                error!("calling poll for {} iter {}",earliest_idx,iteration);
                for i in 0..GIRTH {
                    match &mut subs[i] {
                        Ok(subscription) => {
                            if !assume_connected[earliest_idx]|| 0 == iteration &  ((1<<13)-1) {
                                warn!("rechecking connection for {}",i);
                                assume_connected[i] = subscription.is_connected();
                                //TODO: set this back to false if we have not been geeting any data??
                            }

                        },
                        Err(e) => {warn!("{:?} {:?}",i,e);}
                    }
                }
            }


            let dynamic =
                match &mut subs[earliest_idx] {
                    Ok(subscription) => {
                        if assume_connected[earliest_idx] {
                            poll_aeron_subscription(tx_stream, subscription, &mut actor, now).await
                        } else { //not connected so wait a bit
                            Duration::from_millis(20)
                        }

                    }
                    Err(e) => {
                        error!("Internal error, the subscription should be present: {:?}",e);
                        //moving this out of the way to avoid checking again
                        Duration::from_secs(i32::MAX as u64)
                    }
                };


            if 0 == iteration & ((1 << 12) - 1) {
                error!("done calling poll for {} iter {}",earliest_idx,iteration);
            }
            next_times[earliest_idx] = now + if let Some(fixed) = ROUND_ROBIN { fixed } else { dynamic };

        }
        iteration += 1;
    }
    Ok(())
}