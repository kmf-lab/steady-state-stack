use std::error::Error;
use std::sync::Arc;
use futures_timer::Delay;
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::AtomicBuffer;
use aeron::concurrent::logbuffer::frame_descriptor;
use aeron::concurrent::logbuffer::header::Header;
use aeron::subscription::Subscription;
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::distributed_stream::{SteadyStreamTxBundle, SteadyStreamTxBundleTrait, StreamSessionMessage, StreamTxBundleTrait};
use crate::{SteadyCommander, SteadyState};
use crate::*;
use num_traits::Zero;
use crate::commander_context::SteadyContext;
//  https://github.com/real-logic/aeron/wiki/Best-Practices-Guide

#[derive(Default)]
pub struct AeronSubscribeSteadyState {
    sub_reg_id: Vec<Option<i64>>,
}



// In Aeron, the maximum message length is determined by the term buffer length.
// Specifically, the maximum message size is calculated as the
// lesser of 16 MB or one-eighth of the term buffer length.


pub async fn run<const GIRTH:usize,>(context: SteadyContext
                                     , tx: SteadyStreamTxBundle<StreamSessionMessage,GIRTH>
                                     , aeron_connect: Channel
                                     , stream_id: i32
                                     , aeron:Arc<futures_util::lock::Mutex<Aeron>>
                                     , state: SteadyState<AeronSubscribeSteadyState>) -> Result<(), Box<dyn Error>> {

    let cmd = context.into_monitor([], tx.control_meta_data());
    if cfg!(not(test)) {
        internal_behavior(cmd, tx, aeron_connect, stream_id, aeron, state).await
    } else {
        let te:Vec<_> = tx.iter()
            .map(|f| TestEcho(f.clone()) ).collect();
        let sims:Vec<_> = te.iter()
            .map(|f| f as &dyn IntoSimRunner<_>).collect();
        cmd.simulated_behavior(sims).await
    }
}

// 
// pub async fn run<const GIRTH:usize,>(context: SteadyContext
//                                      , tx: SteadyStreamTxBundle<StreamSessionMessage,GIRTH>
//                                      , aeron_connect: Channel
//                                      , stream_id: i32
//                                      , aeron:Arc<futures_util::lock::Mutex<Aeron>>
//                                      , state: SteadyState<AeronSubscribeSteadyState>) -> Result<(), Box<dyn Error>> {
// 
//     //TODO: need a new special function for this.
//     context.into_monitor([], tx.control_meta_data())
//            .simulated_behavior([&TestEcho(tx)]).await
//         
// }


async fn internal_behavior<const GIRTH:usize,C: SteadyCommander>(mut cmd: C
                                                                 , tx: SteadyStreamTxBundle<StreamSessionMessage,GIRTH>
                                                                 , aeron_channel: Channel
                                                                 , stream_id: i32
                                                                 , aeron: Arc<futures_util::lock::Mutex<Aeron>>
                                                                 , state: SteadyState<AeronSubscribeSteadyState>) -> Result<(), Box<dyn Error>> {

    let mut tx = tx.lock().await;

    let mut state = state.lock(|| AeronSubscribeSteadyState::default()).await;
        //TODO: better data structure required, need a collecion of subs.
        let mut subs: [Result<Subscription, Box<dyn Error>>; GIRTH] = std::array::from_fn(|_| Err("Not Found".into()));

        //ensure right length
        while state.sub_reg_id.len() < GIRTH {
            state.sub_reg_id.push(None);
        }
        //add subscriptions

        {
            let mut aeron = aeron.lock().await;
            //trace!("holding add_subscription lock");
            for f in 0..GIRTH {
                if state.sub_reg_id[f].is_none() { //only add if we have not already done this
                    warn!("adding new sub {} {:?}", f as i32 + stream_id, aeron_channel.cstring() );
                    match aeron.add_subscription(aeron_channel.cstring(), f as i32 + stream_id) {
                        Ok(reg_id) => {
                            warn!("new subscription found: {}",reg_id);
                            state.sub_reg_id[f] = Some(reg_id)},
                        Err(e) => {
                            warn!("Unable to add publication: {:?}",e);
                        }
                    };
                }
            };
            warn!("released add_subscription lock");
        }
        Delay::new(Duration::from_millis(4)).await; //back off so our request can get ready

    // now lookup when the subscriptions are ready
        for f in 0..GIRTH {
            if let Some(id) = state.sub_reg_id[f] {
                subs[f] = loop {
                    let sub = {
                                let mut aeron = aeron.lock().await; //caution other actors need this so do jit
                                warn!("holding find_subscription({}) lock",id);
                                aeron.find_subscription(id)
                             };
                    match sub {
                        Err(e) => {
                            if e.to_string().contains("Awaiting")
                                || e.to_string().contains("not ready") {
                                //important that we do not poll fast while driver is setting up
                                Delay::new(Duration::from_millis(2)).await;
                                if cmd.is_liveliness_stop_requested() {
                                    warn!("stop detected before finding publication");
                                    //we are done, shutdown happened before we could start up.
                                    break Err("Shutdown requested while waiting".into());
                                }
                            } else {

                                warn!("Error finding publication: {:?}", e);
                                break Err(e.into());
                            }
                        },
                        Ok(subscription) => {
                            // Take ownership of the Arc and unwrap it
                            match Arc::try_unwrap(subscription) {
                                Ok(mutex) => {
                                    // Take ownership of the inner Mutex
                                    match mutex.into_inner() {
                                        Ok(subscription) => {
                                            // Successfully extracted the ExclusivePublication
                                            //warn!("unwrap");
                                            break Ok(subscription);
                                        }
                                        Err(_) => panic!("Failed to unwrap Mutex"),
                                    }
                                }
                                Err(_) => panic!("Failed to unwrap Arc. Are there other references?"),
                            }
                        }
                    }
                };
            } else { //only add if we have not already done this
                return Err("Check if Media Driver is running.".into());
            }
        }

        warn!("running subscriber '{:?}' all subscriptions in place",cmd.identity());

        while cmd.is_running(&mut || tx.mark_closed()) {

            //warn!("looping");
            // only poll this often
            let _clean = await_for_all!( cmd.wait_periodic(Duration::from_micros(2)) );

                let mut found_data = false;
                for i in 0..GIRTH {
                        match &mut subs[i] {
                            Ok(sub) => {
                                if tx[i].ready.is_empty() {
                                    let mut no_count = 0;
                                    tx[i].fragment_flush_all(&mut cmd);
                                    let mut remaining_poll = if let Some(s)= tx[i].smallest_space() {
                                                                     s as i32
                                                                } else {
                                                                    tx[i].item_channel.capacity() as i32
                                                                };
                                        let mut local_data = false;
                                        let mut sent_count = 0;
                                        let mut sent_bytes = 0;

                                        let _frags = {
                                            //NOTE: aeron is NOT thread safe so we are forced to lock across the entire app
                                            let mut total = 0;
                                            //each call to this is no more than one full SOCKET_SO_RCVBUF
                                            for _ in 0..16 { //16

                                                let c = {
                                                    let now = Instant::now();
                                                    let stream = &mut tx[i];
                                                    //TODO: we need to wait on this lock butalso track it.
                                           //         let mut _aeron = aeron.lock().await;  //other actors need this so do our work quick
                                                    sub.poll(&mut |buffer: &AtomicBuffer
                                                                           , offset: i32
                                                                           , length: i32
                                                                           , header: &Header| {
                                                        local_data = true;
                                                        let flags = header.flags();
                                                        let is_begin: bool = 0 != (flags & frame_descriptor::BEGIN_FRAG);
                                                        let is_end: bool = 0 != (flags & frame_descriptor::END_FRAG);

                                                        stream.fragment_consume(header.session_id()
                                                                               , buffer.as_sub_slice(offset, length)
                                                                               , is_begin, is_end, now);
                                                        sent_count += 1;
                                                        sent_bytes += length;
                                                    }, remaining_poll)
                                                };
                                                // if z>0 && c>0 {
                                                //     warn!("success at {} found more {} ",z,c);
                                                // }

                                                remaining_poll -= c;
                                                total += c;
                                                if !tx[i].ready.is_empty() || c.is_zero() || remaining_poll.is_zero() {
                                                    tx[i].fragment_flush_ready(&mut cmd);
                                                    break;
                                                }
                                                cmd.relay_stats_smartly();
                                                Delay::new(Duration::from_millis(4)).await;
                                             //   warn!("relay stats here");

                                            }
                                            total
                                        };
                                        if !tx[i].ready.is_empty() {
                                            tx[i].fragment_flush_ready(&mut cmd);
                                        }
                                        if !local_data {
                                            no_count += 1;
                                            yield_now().await;
                                        } else {
                                            no_count = 0;
                                        }
                                        found_data |= local_data;
                                        if  no_count>5 {
                                            //TODO: may not flush all so check again periodicly in our main loop
                                            tx[i].fragment_flush_all(&mut cmd);

                                            continue; //next stream
                                        }


                                    // see break, we only stop when we have no data or no room
                                    /////////////////////////////////

                                } else {
                                    tx[i].fragment_flush_ready(&mut cmd);
                                }
                            }
                            Err(_) => {
                                if let Some(id) = state.sub_reg_id[i] {
                                    let sub = {
                                        let mut aeron = aeron.lock().await; //caution other actors need this so do jit
                                        warn!("holding find_subscription({}) lock",id);
                                        aeron.find_subscription(id)
                                    };
                                    match sub {
                                        Err(e) => {
                                            if e.to_string().contains("Awaiting")
                                                || e.to_string().contains("not ready") {
                                                //important that we do not poll fast while driver is setting up
                                                Delay::new(Duration::from_millis(2)).await;
                                                if cmd.is_liveliness_stop_requested() {
                                                    warn!("stop detected before finding publication");
                                                    return Ok(());
                                                    //we are done, shutdown happened before we could start up.
                                                    //break Err("Shutdown requested while waiting".into());
                                                }
                                            } else {

                                                warn!("Error finding publication: {:?}", e);
                                                break;
                                            }
                                        },
                                        Ok(subscription) => {
                                            // Take ownership of the Arc and unwrap it
                                            match Arc::try_unwrap(subscription) {
                                                Ok(mutex) => {
                                                    // Take ownership of the inner Mutex
                                                    match mutex.into_inner() {
                                                        Ok(_subscription) => {
                                                            // Successfully extracted the ExclusivePublication
                                                           //0 subs[i] = Ok(subscription);
                                                            break;
                                                        }
                                                        Err(_) => panic!("Failed to unwrap Mutex"),
                                                    }
                                                }
                                                Err(_) => panic!("Failed to unwrap Arc. Are there other references?"),
                                            }
                                        }
                                    }


                                }

                               // panic!("{:?}", e); //restart this actor for some reason we do not have subscription
                            }
                        }
                    
                }
                if cmd.is_liveliness_stop_requested() || !found_data {
                    continue;
                } else {
                    cmd.relay_stats_smartly();
                }

        }

    Ok(())
}


//
// #[cfg(test)]
// pub(crate) mod aeron_media_driver_tests {
//     use std::env;
//     use super::*;
//     use crate::distributed::aeron_channel_structs::{Endpoint, MediaType};
//     use crate::distributed::distributed_stream::StreamSimpleMessage;
//     use crate::distributed::aeron_channel_builder::{AeronConfig, AqueTech};
//     use crate::distributed::distributed_builder::AqueductBuilder;
//
//     #[async_std::test]
//     async fn test_bytes_process() {
//           if true {
//               return; //Not running this test at this time.
//           }
//         if std::env::var("GITHUB_ACTIONS").is_ok() {
//             return;
//         }
//         unsafe {
//             env::set_var("TELEMETRY_SERVER_PORT", "9301");
//         }
//         let mut graph = GraphBuilder::for_testing().build(());
//
//         if graph.aeron_md().is_none() {
//             info!("aeron test skipped, no media driver present");
//             return;
//         }
//
//         let channel_builder = graph.channel_builder();
//
//         //NOTE: each stream adds startup time as each transfer term must be tripled and zeroed
//         const STREAMS_COUNT:usize = 1;
//         let (to_aeron_tx,to_aeron_rx) = channel_builder
//             .with_capacity(500)
//             .build_as_stream_bundle::<StreamSimpleMessage,STREAMS_COUNT>( 6);
//
//         let aeron_config = AeronConfig::new()
//             .with_media_type(MediaType::Ipc) //for testing
//             //.with_media_type(MediaType::Udp)
//             //.with_term_length(1024 * 1024 * 4)
//             .use_point_to_point(Endpoint {
//                 ip: "127.0.0.1".parse().expect("Invalid IP address"),
//                 port: 40456,
//             })
//             .build();
//
//
//         let stream_id = 7;
//         to_aeron_rx.build_aqueduct(AqueTech::Aeron(graph.aeron_md(), aeron_config.clone(), stream_id)
//                    , &graph.actor_builder().with_name("SenderTest")
//                    , &mut Threading::Spawn);
//
//         for _i in 0..100 {
//             to_aeron_tx[0].testing_send_frame(&[1, 2, 3, 4, 5]).await;
//             to_aeron_tx[0].testing_send_frame(&[6, 7, 8, 9, 10]).await;
//         }
//
//         for i in 0..STREAMS_COUNT {
//             to_aeron_tx[i].testing_close();
//         }
//
//
//         let (from_aeron_tx,from_aeron_rx) = channel_builder
//             .with_capacity(500)
//             .build_as_stream_bundle::<StreamSessionMessage,STREAMS_COUNT>( 6);
//
//         from_aeron_tx.build_aqueduct(AqueTech::Aeron(graph.aeron_md(), aeron_config.clone(),stream_id)
//                        , & graph.actor_builder().with_name("ReceiverTest")
//                        , &mut Threading::Spawn);
//
//
//         graph.start(); //startup the graph
//
//         warn!("waiting -------------------------");
//         //wait till we see 2 full fragments back
//         from_aeron_rx[0].testing_avail_wait(2).await;
//         warn!("found two");
//         graph.request_stop();
//         //we wait up to the timeout for clean shutdown which is transmission of all the data
//         graph.block_until_stopped(Duration::from_secs(21));
//
//         //from_aeron_rx.
//
//         let mut data = [0u8; 5];
//         for i in 0..100 {
//             let result = from_aeron_rx[0].testing_take_frame(&mut data[0..5]).await;
//             assert_eq!(5, result, "failed on iteration {}", i);
//             assert_eq!([1, 2, 3, 4, 5], data);
//             let result = from_aeron_rx[0].testing_take_frame(&mut data[0..5]).await;
//             assert_eq!(5, result, "failed on iteration {}", i);
//             assert_eq!([6,7,8,9,10], data);
//         }
//
//     }
// }


