use std::error::Error;
use std::sync::{Arc};
use futures_timer::Delay;
use ringbuf::consumer::Consumer;
use steady_state_aeron::aeron::Aeron;
use steady_state_aeron::concurrent::atomic_buffer::{AlignedBuffer, AtomicBuffer};
use steady_state_aeron::exclusive_publication::ExclusivePublication;
use steady_state_aeron::utils::types::Index;
use crate::distributed::aeron_channel::Channel;
use crate::distributed::steady_stream::{SteadyStreamRxBundle, SteadyStreamRxBundleTrait, StreamSimpleMessage, StreamRxBundleTrait};
use crate::{into_monitor, SteadyCommander, SteadyContext, SteadyState};
use crate::*;
use crate::monitor::{RxMetaDataHolder};

#[derive(Default)]
pub(crate) struct AeronPublishSteadyState {
    pub(crate) pub_reg_id: Vec<Option<i64>>,
}

pub async fn run<const GIRTH:usize,>(context: SteadyContext
                                     , rx: SteadyStreamRxBundle<StreamSimpleMessage,GIRTH>
                                     , aeron_connect: Channel
                                     , aeron:Arc<futures_util::lock::Mutex<Aeron>>
                                     , state: SteadyState<AeronPublishSteadyState>) -> Result<(), Box<dyn Error>> {
    internal_behavior(into_monitor!(context, RxMetaDataHolder::new(rx.control_meta_data()), []), rx, aeron_connect, aeron, state).await
}

async fn internal_behavior<const GIRTH:usize,C: SteadyCommander>(mut cmd: C
                                                                 , rx: SteadyStreamRxBundle<StreamSimpleMessage,GIRTH>
                                                                 , aeron_channel: Channel
                                                                 , aeron:Arc<futures_util::lock::Mutex<Aeron>>
                                                                 , state: SteadyState<AeronPublishSteadyState>) -> Result<(), Box<dyn Error>> {

    let mut rx = rx.lock().await;
    let mut state_guard = steady_state(&state, || AeronPublishSteadyState::default()).await;
    if let Some(state) = state_guard.as_mut() {
        let mut pubs: [ Result<ExclusivePublication, Box<dyn Error> >;GIRTH] = std::array::from_fn(|_| Err("Not Found".into())  );
               
        //ensure right length
        while state.pub_reg_id.len()<GIRTH {
            state.pub_reg_id.push(None);
        }

        {
            let mut aeron = aeron.lock().await;  //other actors need this so do our work quick

           //trace!("holding add_exclusive_publication lock");
            for f in 0..GIRTH {
                if state.pub_reg_id[f].is_none() { //only add if we have not already done this
                    warn!("adding new pub {} {:?}",rx[f].stream_id,aeron_channel.cstring() );
                    match aeron.add_exclusive_publication(aeron_channel.cstring(), rx[f].stream_id) {
                        Ok(reg_id) => state.pub_reg_id[f] = Some(reg_id),
                        Err(e) => {
                            warn!("Unable to add publication: {:?}",e);
                        }
                    };
                }
            };
            //trace!("released add_exclusive_publication lock");
        }
        Delay::new(Duration::from_millis(20)).await; //back off so our request can get ready

        // now lookup when the publications are ready
            for f in 0..GIRTH {
                if let Some(id) = state.pub_reg_id[f] {
                    pubs[f] = loop {
                        let ex_pub = {
                            let mut aeron = aeron.lock().await; //other actors need this so jit
                            //trace!("holding find_exclusive_publication({}) lock",id);
                            let response = aeron.find_exclusive_publication(id);
                            //trace!("releasing find_exclusive_publication({}) lock",id);
                            response
                        };
                        match ex_pub {
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
                            Ok(publication) => {
                                // Take ownership of the Arc and unwrap it
                                match Arc::try_unwrap(publication) {
                                    Ok(mutex) => {
                                        // Take ownership of the inner Mutex
                                        match mutex.into_inner() {
                                            Ok(publication) => {
                                                // Successfully extracted the ExclusivePublication
                                                break Ok(publication);
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


        warn!("running publisher '{:?}' all publications in place",cmd.identity());

        let wait_for = 16000;
        let mut backoff = false;
        while cmd.is_running(&mut || rx.is_closed_and_empty()) {
    
            let clean = if backoff {
                await_for_all!(cmd.wait_periodic(Duration::from_millis(100)),
                               cmd.wait_closed_or_avail_message_stream::<StreamSimpleMessage>(&mut rx, wait_for, 1))
            } else {
                await_for_all!(cmd.wait_closed_or_avail_message_stream::<StreamSimpleMessage>(&mut rx, wait_for, 1))
            };

            let mut count_done = 0;

                backoff = false;
                for i in 0..GIRTH {                    
                   
                        while let Some(stream_message) = cmd.try_peek(&mut rx[i].item_channel) {
                           
                                match &mut pubs[i] {
                                    Ok(p) => {
                                        let message_length = stream_message.length as usize;
                                        let (a, b) = rx[i].payload_channel.rx.as_mut_slices(); //TODO: better function?
                                        let a_len = message_length.min(a.len());
                                        let remaining_read = message_length - a_len;

                                        let offer_response = if remaining_read == 0 {
                                            p.offer_part(AtomicBuffer::wrap_slice(&mut a[0..a_len]), 0, message_length as Index)
                                        } else {
                                            // Rare case: we are going over the edge, so we are forced to copy the data
                                            // NOTE: With patch to exclusive_publication, we could avoid this copy
                                            let aligned_buffer = AlignedBuffer::with_capacity(message_length as Index);
                                            let mut buf = AtomicBuffer::from_aligned(&aligned_buffer);

                                            // SAFETY: Ensure that memory regions do not overlap
                                            assert!(a_len + remaining_read <= aligned_buffer.len as usize);

                                            // Use non-overlapping memory copy for safety
                                            buf.put_bytes(0, &a[..a_len]);
                                            //warn!("to read {} and buf now {:?} ",message_length, &buf.as_slice()[..a_len.min(40)]);

                                            let b_len = remaining_read.min(b.len());
                                            let extended_read = remaining_read - b_len;

                                            buf.put_bytes(a_len as Index, &b[..b_len]);

                                            // Check if the entire expected data has been read
                                            assert_eq!(0, extended_read, "Error: not all data was read into buffer!");

                                            // Add detailed warnings for debugging purposes
                                            // warn!("Buffer content (A): {:?}", &buf.as_slice()[..message_length.min(40)]);
                                            p.offer_part(buf, 0, message_length as Index)

                                        };


                                        // const APP_ID:i64 = 8675309;
                                        // //TODO: need to use this field to hash payload with our private key.
                                        // let f: OnReservedValueSupplier = |buf, start, _len| {
                                        //     // Example hash calculation of the data in the buffer
                                        //     //calculate_hash(buf, start, len)
                                        //     APP_ID // + buf.get_bytes(start)
                                        // };
                                        match offer_response {
                                            Ok(_value) => {
                                                count_done += 1;
                                                //worked so we can now finalize the take
                                                let msg = cmd.try_take(&mut rx[i].item_channel);
                                                debug_assert!(msg.is_some());
                                                let actual = cmd.advance_index(&mut rx[i].payload_channel, message_length);
                                                debug_assert_eq!(actual, message_length);
                                            },
                                            Err(aeron_error) => {
                                                //warn!("backoff message {}",aeron_error);
                                                backoff= true;
                                                break; //go on to next stream this one is full
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        panic!("{:?}",e);//we should have had the pup so try again
                                    }
                                }
                                                                      
                        }
                }

            if count_done>0 {
              //  warn!("count done per pass {}",count_done);
            }
         }        
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod aeron_tests {
    use super::*;
    use crate::distributed::aeron_channel::{Endpoint, MediaType};
    use crate::distributed::aeron_distributed::{AeronConfig, DistributedTech};
    use crate::distributed::aeron_subscribe;
    use crate::distributed::steady_stream::{SteadyStreamTxBundle, SteadyStreamTxBundleTrait, StreamSessionMessage, StreamTxBundleTrait};
    use crate::monitor::TxMetaDataHolder;
    use crate::distributed::steady_stream::{LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundleClone, StreamSimpleMessage};

    //NOTE: bump this up for longer running load tests
    pub const TEST_ITEMS: usize = 2_000_000_000;
    pub const STREAM_ID: i32 = 5; 

    pub async fn mock_sender_run<const GIRTH: usize>(mut context: SteadyContext
                                                     , tx: SteadyStreamTxBundle<StreamSimpleMessage, GIRTH>) -> Result<(), Box<dyn Error>> {

        let mut cmd = into_monitor!(context, [], TxMetaDataHolder::new(tx.control_meta_data()));
        let mut tx = tx.lock().await;

        let mut sent_count = 0;
        while cmd.is_running(&mut || tx.mark_closed()) {

            //waiting for at least 1 channel in the stream has room for 2 made of 6 bytes
            let vacant_items = 20000;
            let data_size = 8;
            let vacant_bytes = vacant_items * data_size;

            let _clean = await_for_all!(cmd.wait_shutdown_or_vacant_units_stream(&mut tx, vacant_items, vacant_bytes, 1));
            let remaining = TEST_ITEMS - sent_count;

            if remaining > 0 {
                let actual_vacant = cmd.vacant_units(&mut tx[0].item_channel).min(remaining);
                for _i in 0..(actual_vacant >> 1) {
                    let _result = cmd.try_stream_send(&mut tx, STREAM_ID, &[1, 2, 3, 4, 5, 6, 7, 8]);
                    let _result = cmd.try_stream_send(&mut tx, STREAM_ID, &[9, 10, 11, 12, 13, 14, 15, 16]);
                }
                sent_count += actual_vacant;
            }

            if sent_count>=TEST_ITEMS {
                //if an actor exits without closing its streams we will get a dirty shutdown.
                tx.mark_closed();
                error!("sender is done");
                return Ok(()); //exit now because we sent all our data
            }
        }

        Ok(())
    }

    pub async fn mock_receiver_run<const GIRTH:usize>(mut context: SteadyContext
                                                      , rx: SteadyStreamRxBundle<StreamSessionMessage, GIRTH>) -> Result<(), Box<dyn Error>> {

        let mut cmd = into_monitor!(context, RxMetaDataHolder::new(rx.control_meta_data()), []);
        let mut rx = rx.lock().await;

        let mut received_count = 0;
        while cmd.is_running(&mut || rx.is_closed_and_empty()) {

            let messages_required = 40_000;
            let _clean = await_for_all!(
                                       // cmd.wait_periodic(Duration::from_millis(400)),
                                        cmd.wait_closed_or_avail_message_stream(&mut rx, messages_required,1));

            //we waited above for 2 messages so we know there are 2 to consume
            //reading from a single channel with a single stream id

            let avail = cmd.avail_units(&mut rx[0].item_channel);

            for i in 0..(avail>>1) {
                if let Some(d) = cmd.try_take_stream(&mut rx[0]) {
                    //warn!("test data {:?}",d.payload);
                    assert_eq!(&*Box::new([1, 2, 3, 4, 5, 6, 7, 8]), &*d.payload);
                }
                if let Some(d) = cmd.try_take_stream(&mut rx[0]) {
                    //warn!("test data {:?}",d.payload);
                    assert_eq!(&*Box::new([9, 10, 11, 12, 13, 14, 15, 16]), &*d.payload);
                }
            }
            received_count += messages_required;
            cmd.relay_stats_smartly(); //should not be needed.

            //here we request shutdown but we only leave after our upstream actors are done
            if received_count >= (TEST_ITEMS-messages_required) {
                error!("stop requested");
                cmd.request_graph_stop();
                return Ok(());
            }
        }

        error!("receiver is done");
        Ok(())
    }

    #[async_std::test]
    async fn test_bytes_process() {
        let _lock = aeron_subscribe::aeron_media_driver_tests::TEST_MUTEX.lock().await; // Lock to ensure sequential execution

        let mut graph = GraphBuilder::for_testing()
            .with_telemetry_metric_features(true)
            .build(());

        if !graph.is_aeron_media_driver_present() {
            info!("aeron test skipped, no media driver present");
            return;
        }

        let channel_builder = graph.channel_builder();

        let (to_aeron_tx,to_aeron_rx) = channel_builder
            .with_avg_rate()
            .with_avg_filled()
            .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
            .build_as_stream::<StreamSimpleMessage,1>(STREAM_ID
                                                                  , 911937
                                                                  , 9119370);
        let (from_aeron_tx,from_aeron_rx) = channel_builder
            .with_avg_rate()
            .with_avg_filled()
            .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
            .build_as_stream::<StreamSessionMessage,1>(STREAM_ID
                                                                   , 937197
                                                                   , 9371970);

        let aeron_config = AeronConfig::new()            
            .with_media_type(MediaType::Udp)
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("Invalid IP address"),
                port: 40456,
            })
            
            
            .with_term_length(1024 * 1024 * 32)
            .build(); 
        let dist =  DistributedTech::Aeron(aeron_config);


      //let this create the pipe first??
        graph.build_stream_collector(dist.clone()
                                     , "ReceiverTest"
                                     , from_aeron_tx
                                     , &mut Threading::Spawn);

        graph.build_stream_distributor(dist.clone()
                                       , "SenderTest"
                                       , to_aeron_rx
                                       , &mut Threading::Spawn);

        graph.actor_builder().with_name("MockSender")
            .with_mcpu_avg()
            .with_thread_info()
            .build(move |context| mock_sender_run(context, to_aeron_tx.clone())
                   , &mut Threading::Spawn);

        graph.actor_builder().with_name("MockReceiver")
            .with_mcpu_avg()
            .with_thread_info()
            .build(move |context| mock_receiver_run(context, from_aeron_rx.clone())
                   , &mut Threading::Spawn);

        graph.start(); //startup the graph

        graph.block_until_stopped(Duration::from_secs(21));

    }

}
