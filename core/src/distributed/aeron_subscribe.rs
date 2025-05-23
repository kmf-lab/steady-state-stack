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
use crate::distributed::distributed_stream::{SteadyStreamTx, StreamSessionMessage};
use crate::{SteadyCommander, SteadyState, StreamTx, SimTx};
use crate::commander_context::SteadyContext;
use crate::core_tx::TxCore;
use crate::distributed::polling;
use crate::yield_now;

#[derive(Default)]
pub struct AeronSubscribeSteadyState {
    sub_reg_id: Option<i64>,
}

pub async fn run(
    context: SteadyContext,
    tx: SteadyStreamTx<StreamSessionMessage>,
    aeron_connect: Channel,
    stream_id: i32,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let mut cmd = context.into_monitor([], [&tx]);
    if cmd.use_internal_behavior {
        while cmd.aeron_media_driver().is_none() {
            warn!("unable to find Aeron media driver, will try again in 15 sec");
            let mut tx = tx.lock().await;
            if cmd.is_running(&mut || tx.mark_closed()) {
                cmd.wait_periodic(Duration::from_secs(15)).await;
            } else {
                return Ok(());
            }
        }
        let aeron_media_driver = cmd.aeron_media_driver().expect("media driver");
        internal_behavior(cmd, tx, aeron_connect, stream_id, aeron_media_driver, state).await
    } else {
        cmd.simulated_behavior(vec![&SimTx(tx)]).await
    }
}

async fn internal_behavior<C: SteadyCommander>(
    mut cmd: C,
    tx: SteadyStreamTx<StreamSessionMessage>,
    aeron_channel: Channel,
    stream_id: i32,
    aeron: Arc<futures_util::lock::Mutex<Aeron>>,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let mut tx = tx.lock().await; // Lock tx at the start, as in the original
    let mut state = state.lock(|| AeronSubscribeSteadyState::default()).await;
    let mut sub: Option<Subscription> = None;

    // Step 1: Add subscription if not already registered
    if state.sub_reg_id.is_none() {
        let mut aeron_guard = aeron.lock().await;
        let reg_id = aeron_guard.add_subscription(aeron_channel.cstring(), stream_id)?;
        warn!("new subscription registered: {}", reg_id);
        state.sub_reg_id = Some(reg_id);
    }

    // Step 2: Wait for the subscription to become available
    while sub.is_none() {
        if let Some(id) = state.sub_reg_id {
            let mut aeron_guard = aeron.lock().await;
            match aeron_guard.find_subscription(id) {
                Ok(subscription) => {
                    match Arc::try_unwrap(subscription) {
                        Ok(mutex) => {
                            match mutex.into_inner() {
                                Ok(subscription) => {
                                    sub = Some(subscription);
                                },
                                Err(_) => panic!("Failed to unwrap Mutex"),
                            }
                        },
                        Err(_) => panic!("Failed to unwrap Arc. Are there other references?"),
                    }
                }
                Err(e) => {
                    if e.to_string().contains("Awaiting") || e.to_string().contains("not ready") {
                        Delay::new(Duration::from_millis(2)).await;
                        if cmd.is_liveliness_stop_requested() {
                            warn!("shutdown requested while waiting for subscription");
                            return Ok(());
                        }
                    } else {
                        return Err(format!("Error finding subscription: {:?}", e).into());
                    }
                }
            }
        } else {
            return Err("Subscription registration ID not set".into());
        }
    }

    let mut sub = sub.unwrap();
    let mut next_poll_time = Instant::now();

    // Step 3: Main polling loop
    error!("running subscriber '{:?}' with subscription in place", cmd.identity());
    while cmd.is_running(&mut || tx.mark_closed()) {
        let now = Instant::now();
        if now < next_poll_time {
            cmd.wait_periodic(next_poll_time - now).await;
        }

        if !tx.shared_is_full() {
            let delay = poll_aeron_subscription(&mut tx, &mut sub, &mut cmd).await;
            next_poll_time = now + delay;
        } else {
            let fastest_duration = tx.fastest_byte_processing_duration();
            next_poll_time = now + if let Some(f) = fastest_duration {
                f * (tx.capacity() >> 1) as u32
            } else {
                tx.max_poll_latency
            };
        }
    }

    Ok(())
}

async fn poll_aeron_subscription<C: SteadyCommander>(
    tx: &mut StreamTx<StreamSessionMessage>,
    sub: &mut Subscription,
    cmd: &mut C,
) -> Duration {
    let mut input_bytes: u32 = 0;
    let mut input_frags: u32 = 0;
    let now = Instant::now();

    let measured_vacant_items = tx.item_channel.shared_vacant_units() as u32;
    let measured_vacant_bytes = tx.payload_channel.shared_vacant_units() as u32;

    loop {
        // tx is already locked by the caller (internal_behavior), so we use it directly
        let remaining_poll = tx.smallest_space().unwrap_or(tx.item_channel.capacity()) as i32;
        if 0 >= sub.poll(
            &mut |buffer: &AtomicBuffer, offset: i32, length: i32, header: &Header| {
                let flags = header.flags();
                let is_begin = (flags & frame_descriptor::BEGIN_FRAG) != 0;
                let is_end = (flags & frame_descriptor::END_FRAG) != 0;
                tx.fragment_consume(header.session_id(), buffer.as_sub_slice(offset, length), is_begin, is_end, now);
                input_bytes += length as u32;
                input_frags += 1;
            },
            remaining_poll,
        ) {
            break; //we got no data so leave now we will try again later
        }
        yield_now().await; //do not remove in many cases this allows us more data in this pass
    }

    // Flush ready messages if any
    if !tx.ready_msg_session.is_empty() {
        let (now_sent_messages, now_sent_bytes) = tx.fragment_flush_ready(cmd);
        let (stored_vacant_items, stored_vacant_bytes) = tx.get_stored_vacant_values();

        if stored_vacant_items > measured_vacant_items as i32 {
            let duration = now.duration_since(tx.last_output_instant);
            tx.store_output_data_rate(
                duration,
                (stored_vacant_items - (measured_vacant_items as i32)) as u32,
                (stored_vacant_bytes - (measured_vacant_bytes as i32)) as u32,
            );
            tx.last_output_instant = now;
            tx.set_stored_vacant_values(
                measured_vacant_items as i32 - now_sent_messages as i32,
                measured_vacant_bytes as i32 - now_sent_bytes as i32,
            );
        }
    }

    // Update input data rate if fragments were received
    if input_frags > 0 {
        let duration = now.duration_since(tx.last_input_instant);
        tx.store_input_data_rate(duration, input_frags, input_bytes);
        tx.last_input_instant = now;
    }

    // Calculate next poll time using the scheduler
    let (avg, std) = tx.guess_duration_between_arrivals();
    let (min, max) = tx.next_poll_bounds();
    let mut scheduler = polling::PollScheduler::new();
    scheduler.set_max_delay_ns(max.as_nanos() as u64);
    scheduler.set_min_delay_ns(min.as_nanos() as u64);
    scheduler.set_std_dev_ns(std.as_nanos() as u64);
    scheduler.set_expected_moment_ns(avg.as_nanos() as u64);
    let now_ns = now.duration_since(tx.last_input_instant).as_nanos() as u64;
    Duration::from_nanos(scheduler.compute_next_delay_ns(now_ns))
}

#[cfg(test)]
pub(crate) mod aeron_media_driver_tests {
    use std::thread::sleep;
    use log::info;
    use super::*;
    use crate::distributed::aeron_channel_structs::{Endpoint, MediaType};
    use crate::distributed::distributed_stream::StreamSimpleMessage;
    use crate::distributed::aeron_channel_builder::{AeronConfig, AqueTech};
    use crate::distributed::aeron_publish::aeron_tests::STREAM_ID;
    use crate::distributed::distributed_builder::AqueductBuilder;
    use crate::{GraphBuilder, Threading};

    #[test]
    #[cfg(not(windows))]
    fn test_bytes_process() {
        // if std::env::var("GITHUB_ACTIONS").is_ok() {
        //     return;
        // }

        let mut graph = GraphBuilder::for_testing().build(());
        if let Some(md) = graph.aeron_media_driver() {
            if let Some(md) = md.try_lock() {
                info!("Found MediaDriver cnc:{:?}",md.context().cnc_file_name()  );
            };
        } else {
            info!("aeron test skipped, no media driver present");
            return;
        }

        let channel_builder = graph.channel_builder();

        //NOTE: each stream adds startup time as each transfer term must be tripled and zeroed
        const STREAMS_COUNT:usize = 1;
        let (to_aeron_tx,to_aeron_rx) = channel_builder
            .with_capacity(500)
            .build_stream_bundle::<StreamSimpleMessage,STREAMS_COUNT>( 6);

        let aeron_config = AeronConfig::new()
            .with_media_type(MediaType::Ipc) //for testing
            //.with_media_type(MediaType::Udp)
            //.with_term_length(1024 * 1024 * 4)
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("Invalid IP address"),
                port: 40456,
            })
            .build();

        //in simulated graph we will build teh same but expect this to be a simulation !!
        to_aeron_rx.build_aqueduct(AqueTech::Aeron(aeron_config.clone(), STREAM_ID)
                               , &graph.actor_builder().with_name("SenderTest").never_simulate(true)
                               , &mut Threading::Spawn);

        for _i in 0..100 {
            to_aeron_tx[0].testing_send_frame(&[1, 2, 3, 4, 5]);
            to_aeron_tx[0].testing_send_frame(&[6, 7, 8, 9, 10]);
        }

        for i in 0..STREAMS_COUNT {
            to_aeron_tx[i].testing_close();
        }

        let (from_aeron_tx,from_aeron_rx) = channel_builder
            .with_capacity(500)
            .build_stream_bundle::<StreamSessionMessage,STREAMS_COUNT>(6);

        //do not simulate yet the main graph will simulate. cfg!(test)
        from_aeron_tx.build_aqueduct(AqueTech::Aeron(aeron_config, STREAM_ID)
                                     , &graph.actor_builder().with_name( "ReceiverTest").never_simulate(true)
                                     , &mut Threading::Spawn);
        graph.start();
        //from_aeron_rx[0].testing_avail_wait(200, Duration::from_secs(20));
        graph.request_stop();
        graph.block_until_stopped(Duration::from_secs(2));

        //let mut data = [0u8; 5];
        for i in 0..100 {
            // let data1: Vec<u8> = vec![1, 2, 3, 4, 5];
            // assert_steady_rx_eq_take!(from_aeron_rx[0], vec!(
            //     //TODO: instant is a big problme for equals.
            //     (StreamSessionMessage::new(5,1,Instant::now(), Instant::now()),data1.into_boxed_slice())));
            // let data2: Vec<u8> = vec![6, 7, 8, 9, 10];
            // assert_steady_rx_eq_take!(from_aeron_rx[0], vec!(
            //     (StreamSessionMessage::new(5,1,Instant::now(), Instant::now()),data2.into_boxed_slice())));


        }
        
    }
}


