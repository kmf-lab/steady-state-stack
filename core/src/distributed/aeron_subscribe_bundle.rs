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
use log::{trace, warn};
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::distributed_stream::{SteadyStreamTxBundle, StreamSessionMessage};
use crate::{IntoSimRunner, SteadyCommander, SteadyState, SteadyStreamTxBundleTrait, StreamTxBundleTrait, TestEcho};
use crate::commander_context::SteadyContext;
use crate::distributed::scheduling::PerpetualScheduler;

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

async fn internal_behavior<const GIRTH: usize, C: SteadyCommander>(
    mut cmd: C,
    tx: SteadyStreamTxBundle<StreamSessionMessage, GIRTH>,
    aeron_channel: Channel,
    stream_id: i32,
    aeron: Arc<futures_util::lock::Mutex<Aeron>>,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let tx_bundle = tx; // Original bundle, assumed cloneable (e.g., Arc-based)
    let mut tx_guards = tx_bundle.lock().await; // Initial lock if needed for setup
    let mut state = state.lock(|| AeronSubscribeSteadyState::default()).await;
    let mut subs: [Result<Subscription, Box<dyn Error>>; GIRTH] = std::array::from_fn(|_| Err("Not Found".into()));

    // Ensure right length
    while state.sub_reg_id.len() < GIRTH {
        state.sub_reg_id.push(None);
    }

    // Add subscriptions
    {
        for f in 0..GIRTH {
            if state.sub_reg_id[f].is_none() { // Only add if we have not already done this
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
    }
    Delay::new(Duration::from_millis(4)).await; // Back off so our request can get ready

    // Now lookup when the subscriptions are ready
    for f in 0..GIRTH {
        if let Some(id) = state.sub_reg_id[f] {
            let mut found = false;
            while cmd.is_running(&mut || tx_guards.mark_closed()) && !found {
                let sub = {
                    aeron.lock().await.find_subscription(id)
                };
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

    trace!("running subscriber '{:?}' all subscriptions in place", cmd.identity());

    // Wrap cmd in Arc<Mutex> for shared mutable access
    let cmd_arc = Arc::new(Mutex::new(cmd));
    let tx_bundle_clone = tx_bundle.clone(); // Clone for closure

    let mut scheduler = PerpetualScheduler::new(
        [Duration::from_millis(0); GIRTH], // Initial delays set to 0 for immediate start
        move |idx| {
            let cmd_arc = cmd_arc.clone();
            let tx_bundle = tx_bundle_clone.clone();
            async move {
                let mut tx_item = tx_bundle[idx].lock().await; // Lock specific mutex
                let mut cmd = cmd_arc.lock().await; // Lock cmd
                if let Ok(sub) = &mut subs[idx] {
                    let now = Instant::now();
                    let remaining_poll = if let Some(s) = tx_item.smallest_space() {
                        s
                    } else {
                        tx_item.item_channel.capacity()
                    };
                    let mut sent_count = 0;
                    let mut sent_bytes = 0;
                    let frags = sub.poll(&mut |buffer: &AtomicBuffer,
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
                        sent_count += 1;
                        sent_bytes += length;
                    }, remaining_poll as i32);

                    if frags > 0 || !tx_item.ready_msg_session.is_empty() {
                        tx_item.fragment_flush_ready(&mut cmd);
                    }
                    if frags > 0 {
                        if let Some(l) = tx_item.last_data_instant {
                            let _avg_period = now.duration_since(l).div(frags as u32);
                            // TODO: add to power of 2 ring buffer for running average
                        }
                        tx_item.last_data_instant = Some(now);
                    }
                }
                Duration::from_millis(10)
            }
        },
    );

    let mut now = Instant::now();
    loop {
        {
            let mut cmd = cmd_arc.lock().await;
            if !cmd.is_running(&mut || tx_guards.mark_closed()) {
                break;
            }
        }
        now = scheduler.run_single_pass(now).await; // Add error handling if needed
    }

    Ok(())
}