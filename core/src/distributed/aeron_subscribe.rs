use std::collections::HashMap;
use std::error::Error;
use std::sync::{Arc};
use async_ringbuf::AsyncRb;
use futures_timer::Delay;
use steady_state_aeron::aeron::Aeron;
use steady_state_aeron::concurrent::atomic_buffer::AtomicBuffer;
use steady_state_aeron::concurrent::logbuffer::frame_descriptor;
use steady_state_aeron::concurrent::logbuffer::header::Header;
use steady_state_aeron::image::ControlledPollAction;
use steady_state_aeron::subscription::Subscription;
use crate::distributed::aeron_channel::Channel;
use crate::distributed::steady_stream::{SteadyStreamTxBundle, SteadyStreamTxBundleTrait, StreamTxBundleTrait, StreamSessionMessage};
use crate::{into_monitor, SteadyCommander, SteadyContext, SteadyState};
use crate::*;
use ringbuf::storage::Heap;
use async_ringbuf::traits::Split;
use async_ringbuf::wrap::AsyncWrap;
use crate::monitor::{TxMetaDataHolder};
use ahash::AHashMap;

#[derive(Default)]
pub(crate) struct AeronSubscribeSteadyState {
    sub_reg_id: Vec<Option<i64>>,
}

fn test() {
   let rb = AsyncRb::<Heap<u8>>::new(1000).split();
   let mut p: HashMap<i32, (AsyncWrap<Arc<AsyncRb<Heap<u8>>>, true, false>, AsyncWrap<Arc<AsyncRb<Heap<u8>>>,false, true>) > = HashMap::default();

   p.insert(123 as i32, rb);


    let mut map: AHashMap<i32, (AsyncWrap<Arc<AsyncRb<Heap<u8>>>, true, false>, AsyncWrap<Arc<AsyncRb<Heap<u8>>>,false, true>)> = AHashMap::new();

  //TODO: make channel have TERM const we can use here for async buffer
}

// In Aeron, the maximum message length is determined by the term buffer length.
// Specifically, the maximum message size is calculated as the
// lesser of 16 MB or one-eighth of the term buffer length.

pub async fn run<const GIRTH:usize,>(context: SteadyContext
                                     , tx: SteadyStreamTxBundle<StreamSessionMessage,GIRTH>
                                     , aeron_connect: Channel
                                     , aeron:Arc<futures_util::lock::Mutex<Aeron>>
                                     , state: SteadyState<AeronSubscribeSteadyState>) -> Result<(), Box<dyn Error>> {
    internal_behavior(into_monitor!(context, [], TxMetaDataHolder::new(tx.control_meta_data())), tx, aeron_connect, aeron, state).await
}

async fn internal_behavior<const GIRTH:usize,C: SteadyCommander>(mut cmd: C
                                                                 , tx: SteadyStreamTxBundle<StreamSessionMessage,GIRTH>
                                                                 , aeron_channel: Channel
                                                                 , aeron: Arc<futures_util::lock::Mutex<Aeron>>
                                                                 , state: SteadyState<AeronSubscribeSteadyState>) -> Result<(), Box<dyn Error>> {

    let mut tx = tx.lock().await;

    let mut state_guard = steady_state(&state, || AeronSubscribeSteadyState::default()).await;
    if let Some(state) = state_guard.as_mut() {
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
                    warn!("adding new sub {} {:?}", tx[f].stream_id, aeron_channel.cstring() );
                    match aeron.add_subscription(aeron_channel.cstring(), tx[f].stream_id) {
                        Ok(reg_id) => state.sub_reg_id[f] = Some(reg_id),
                        Err(e) => {
                            warn!("Unable to add publication: {:?}",e);
                        }
                    };
                }
            };
            //trace!("released add_subscription lock");
        }
        Delay::new(Duration::from_millis(40)).await; //back off so our request can get ready

    // now lookup when the subscriptions are ready
        for f in 0..GIRTH {
            if let Some(id) = state.sub_reg_id[f] {
                subs[f] = loop {
                    let sub = {
                        let mut aeron = aeron.lock().await; //caution other actors need this so do jit
                        // trace!("holding find_subscription({}) lock",id);
                        let response = aeron.find_subscription(id);
                        // trace!("released find_subscription({}) lock",id);
                        response
                    };
                    match sub {
                        Err(e) => {
                            if e.to_string().contains("Awaiting")
                                || e.to_string().contains("not ready") {
                                //important that we do not poll fast while driver is setting up
                                Delay::new(Duration::from_millis(40)).await;
                                if cmd.is_liveliness_stop_requested() {
                                    //trace!("stop detected before finding publication");
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
     
            // only poll this often
            let clean = await_for_all!( cmd.wait_periodic(Duration::from_micros(200)) );


            const MIN_SPACE: usize = 64;

            //stay looping until we find a pass with no data.
            loop {
                let mut found_data = false;
                for i in 0..GIRTH {
                    let mut max_poll_frags = tx[i].item_channel.vacant_units();
                    if max_poll_frags > MIN_SPACE {
                        match &mut subs[i] {
                            Ok(ref mut sub) => {
                                if tx[i].ready.is_none() {
                                    ////////////////////////
                                    // stay here and poll as long as we can
    
                                    
                                    if !sub.is_connected() {
                                        if sub.is_closed() {
                                            //confirm this is a good time to shut down??
                                            //TODO: then total this across all streams
                                        }
                                    }

                                    let mut no_count = 0;
                                    loop {
                                        let mut local_data = false;
                                        sub.controlled_poll(&mut |buffer: &AtomicBuffer
                                                                  , offset: i32
                                                                  , length: i32
                                                                  , header: &Header| {
                                            local_data = true;
                                            if tx[i].ready.is_none() { //only take data if we flushed last ready
                                                let is_begin: bool = 0 != (header.flags() & frame_descriptor::BEGIN_FRAG);
                                                let is_end: bool = 0 != (header.flags() & frame_descriptor::END_FRAG);
                                                tx[i].fragment_consume(&mut cmd, header.session_id(), buffer.as_sub_slice(offset, length), is_begin, is_end);
                                                Ok(ControlledPollAction::CONTINUE)
                                            } else {
                                                tx[i].fragment_flush(&mut cmd);
                                                Ok(ControlledPollAction::ABORT)
                                            }
                                        }, max_poll_frags as i32);
                                        if !local_data {
                                            no_count += 1;
                                        }
                                        found_data |= local_data;
                                        cmd.relay_stats_smartly();
                                        if  no_count>4 || tx[i].ready.is_some() {
                                            break; //leave loop since there was no data or no room
                                        }
                                        max_poll_frags = tx[i].item_channel.vacant_units();
                                        cmd.relay_stats_smartly();
                                    }

                                    // see break, we only stop when we have no data or no room
                                    /////////////////////////////////

                                } else {
                                    tx[i].fragment_flush(&mut cmd);
                                }
                            }
                            Err(e) => {
                                panic!("{:?}", e); //restart this actor for some reason we do not have subscription
                            }
                        }
                    }
                }
                if cmd.is_liveliness_stop_requested() || !found_data {
                    break;
                } else {
                    cmd.relay_stats_smartly();
                }
            }

        }
    }
    Ok(())
}



#[cfg(test)]
pub(crate) mod aeron_media_driver_tests {
    use std::net::{IpAddr, Ipv4Addr};
    use super::*;
    use crate::distributed::aeron_channel::{Endpoint, MediaType};
    use crate::distributed::steady_stream::StreamSimpleMessage;
    use async_std::sync::Mutex;
    use once_cell::sync::Lazy;
    use crate::distributed::aeron_distributed::{AeronConfig, DistributedTech};

    pub(crate) static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));


    #[async_std::test]
    async fn test_bytes_process() {
        let _lock = TEST_MUTEX.lock().await; // Lock to ensure sequential execution

        let mut graph = GraphBuilder::for_testing()
            .with_telemetry_metric_features(false)
            .build(());

        if !graph.is_aeron_media_driver_present() {
            info!("aeron test skipped, no media driver present");
            return;
        }

        let channel_builder = graph.channel_builder();

        //NOTE: each stream adds startup time as each transfer term must be tripled and zeroed
        const STREAMS_COUNT:usize = 1;
        let (to_aeron_tx,to_aeron_rx) = channel_builder
            .build_as_stream::<StreamSimpleMessage,STREAMS_COUNT>(0, 500, 3000);
        let (from_aeron_tx,from_aeron_rx) = channel_builder
            .build_as_stream::<StreamSessionMessage,STREAMS_COUNT>(0, 500, 3000);

        let aeron_config = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("Invalid IP address"),
                port: 40456,
            })
            .with_term_length(1024 * 1024 * 4)
            .build();

        let dist =  DistributedTech::Aeron(aeron_config);

        graph.build_stream_collector(dist.clone()
                                     , "ReceiverTest"
                                     , from_aeron_tx
                                     , &mut Threading::Spawn);

        graph.build_stream_distributor(dist.clone()
                                         , "SenderTest"
                                         , to_aeron_rx
                                         , &mut Threading::Spawn);

        for i in 0..100 {
            to_aeron_tx[0].testing_send_frame(&[1, 2, 3, 4, 5]).await;
            to_aeron_tx[0].testing_send_frame(&[6, 7, 8, 9, 10]).await;
        }
        
        for i in 0..STREAMS_COUNT {
            to_aeron_tx[i].testing_close().await;
        }
        

        graph.start(); //startup the graph

        //wait till we see 2 full fragments back
        from_aeron_rx[0].testing_avail_wait(2).await;

        graph.request_stop();
        //we wait up to the timeout for clean shutdown which is transmission of all the data
        graph.block_until_stopped(Duration::from_secs(4));

        //from_aeron_rx.

        let mut data = [0u8; 5];
        for i in 0..100 {
            let result = from_aeron_rx[0].testing_take_frame(&mut data[0..5]).await;
            assert_eq!(5, result, "failed on iteration {}", i);
            assert_eq!([1, 2, 3, 4, 5], data);
            let result = from_aeron_rx[0].testing_take_frame(&mut data[0..5]).await;
            assert_eq!(5, result, "failed on iteration {}", i);
            assert_eq!([6,7,8,9,10], data);
        }
        
    }
}


