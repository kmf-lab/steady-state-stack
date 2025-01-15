use std::error::Error;
use std::sync::{Arc};
use futures_timer::Delay;
use ringbuf::consumer::Consumer;
use steady_state_aeron::aeron::Aeron;
use steady_state_aeron::concurrent::atomic_buffer::{AlignedBuffer, AtomicBuffer};
use steady_state_aeron::exclusive_publication::ExclusivePublication;
use steady_state_aeron::utils::types::Index;
use crate::distributed::aeron_channel::Channel;
use crate::distributed::steady_stream::{SteadyStreamRxBundle, SteadyStreamRxBundleTrait, StreamMessage, StreamRxBundleTrait};
use crate::{into_monitor, SteadyCommander, SteadyContext, SteadyState};
use crate::*;
use crate::monitor::{RxMetaDataHolder};

#[derive(Default)]
pub(crate) struct AeronPublishSteadyState {
    pub(crate) pub_reg_id: Vec<Option<i64>>,
}

pub async fn run<const GIRTH:usize,>(context: SteadyContext
                                     , rx: SteadyStreamRxBundle<StreamMessage,GIRTH>
                                     , aeron_connect: Channel
                                     , aeron:Arc<futures_util::lock::Mutex<Aeron>>
                                     , state: SteadyState<AeronPublishSteadyState>) -> Result<(), Box<dyn Error>> {
    let rx_meta = RxMetaDataHolder::new(rx.payload_meta_data());
    internal_behavior(into_monitor!(context, rx_meta, []), rx, aeron_connect, aeron, state).await
}

async fn internal_behavior<const GIRTH:usize,C: SteadyCommander>(mut cmd: C
                                                                 , rx: SteadyStreamRxBundle<StreamMessage,GIRTH>
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
            warn!("holding add_exclusive_publication lock");
            for f in 0..GIRTH {
                if state.pub_reg_id[f].is_none() { //only add if we have not already done this
                    warn!("adding new pub {}",rx[f].stream_id);
                    match aeron.add_exclusive_publication(aeron_channel.cstring(), rx[f].stream_id) {
                        Ok(reg_id) => state.pub_reg_id[f] = Some(reg_id),
                        Err(e) => {
                            warn!("Unable to add publication: {:?}",e);
                        }
                    };
                }
            };
            warn!("released add_exclusive_publication lock");
        }
        Delay::new(Duration::from_millis(2)).await; //back off so our request can get ready

        // now lookup when the publications are ready
            for f in 0..GIRTH {
                if let Some(id) = state.pub_reg_id[f] {
                    pubs[f] = loop {
                        let ex_pub = {
                            let mut aeron = aeron.lock().await; //other actors need this so jit
                            warn!("holding find_exclusive_publication({}) lock",id);
                            let response = aeron.find_exclusive_publication(id);
                            warn!("releasing find_exclusive_publication({}) lock",id);
                            response
                        };
                        match ex_pub {
                            Err(e) => {
                                if e.to_string().contains("Awaiting")
                                    || e.to_string().contains("not ready") {
                                    yield_now::yield_now().await;                                     
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

        warn!("running publisher");
        while cmd.is_running(&mut || rx.is_closed_and_empty()) {
    
            let clean = await_for_all!(cmd.wait_closed_or_avail_units_stream::<StreamMessage>(&mut rx, 1,1));
            if clean {
                for i in 0..GIRTH {                    
                   
                        if let Some(stream_message) = cmd.try_peek(&mut rx[i].control_channel) {
                           
                                match &mut pubs[i] {
                                    Ok(p) => {
                                        let to_read = stream_message.length as usize;
                                        let (a, b) = rx[i].payload_channel.rx.as_mut_slices(); //TODO: better function?
                                        let a_len = to_read.min(a.len());
                                        let remaining_read = to_read - a_len;
                                        
                                        let to_send = if 0 == remaining_read {
                                            //trace!("simple wrap slice {}",a_len);
                                            AtomicBuffer::wrap_slice(&mut a[0..a_len])
                                        } else {
                                            //rare: we are going over the edge so we are forced to copy the data
                                            //NOTE: with patch to exclusive_publication we could avoid this copy
                                            let aligned_buffer = AlignedBuffer::with_capacity(to_read as Index);
                                            let buf = AtomicBuffer::from_aligned(&aligned_buffer);
                                            buf.put_bytes(0, &mut a[0..a_len]);
                                            let b_len = remaining_read.min(b.len());
                                            let extended_read = remaining_read - b_len;
                                            buf.put_bytes(a_len as Index, &mut b[0..b_len]);
                                            assert_eq!(0, extended_read); //we should have read all the data
                                            warn!("tested rare copy buffer {} {}",a_len,b_len);
                                            buf
                                        };
                                        
                                        // const APP_ID:i64 = 8675309;
                                        // //TODO: need to use this field to hash payload with our private key.
                                        // let f: OnReservedValueSupplier = |buf, start, _len| {
                                        //     // Example hash calculation of the data in the buffer
                                        //     //calculate_hash(buf, start, len)
                                        //     APP_ID // + buf.get_bytes(start)
                                        // };
                                        let offer_response = p.offer_part(to_send, 0, to_send.capacity());
                                        match offer_response {
                                            Ok(value) => {
                                                let slice = to_send.as_slice();
                                                warn!("Published FirstByte:{:?} Len:{:?} LastByte:{:?} NewStreamPos:{}", slice[0], slice.len(),slice[slice.len()-1], value);
                                                //worked so we can now finalize the take
                                                let msg = cmd.try_take(&mut rx[i].control_channel);
                                                debug_assert!(msg.is_some());
                                                let actual = cmd.advance_index(&mut rx[i].payload_channel,to_read);
                                                debug_assert_eq!(actual,to_read);
                                               
                                            },
                                            Err(_aeron_error) => {
                                                //trace!("Trying again, Error publishing data: {:?}", aeron_error);
                                                yield_now::yield_now().await;
                                                //unable to publish
                                                //we only peeked however so break and do it later
                                                
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        panic!("{:?}",e);//we should have had the pup so try again
                                    }
                                }
                                                                      
                        } else {
                             //next one
                        }
                    
                }
            }        
         }        
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod aeron_tests {
    use std::net::{IpAddr, Ipv4Addr};
    use futures_timer::Delay;
    use super::*;
    use crate::distributed::aeron_channel::MediaType;
    use crate::distributed::aeron_distributed::DistributionBuilder;
    use crate::distributed::steady_stream::{SteadyStreamTxBundle, SteadyStreamTxBundleTrait, StreamFragment, StreamTxBundleTrait};
    use crate::monitor::TxMetaDataHolder;

    pub async fn mock_sender_run<const GIRTH: usize>(mut context: SteadyContext
                                                     , tx: SteadyStreamTxBundle<StreamMessage, GIRTH>) -> Result<(), Box<dyn Error>> {

        let tx_meta = TxMetaDataHolder::new(tx.payload_meta_data());
        let mut cmd = into_monitor!(context, [], tx_meta);

        let mut tx = tx.lock().await;
        //normally would be a while but for this test we only need to send these two.
        //TODO: send MB of data..
        if cmd.is_running(&mut || tx.mark_closed()) {

            // let stream_id = 7; TODO: missing methods
            // await_for_all!(cmd.wait_shutdown_or_vacant_aqueduct(&mut tx, 2, 1000));
            // let _ = cmd.try_aqueduct_send(&mut tx, stream_id, &[1,2,3]);
            // let _ = cmd.try_aqueduct_send(&mut tx, stream_id, &[4,5,6]);
        }

        tx.mark_closed();

        Ok(())
    }

    pub async fn mock_receiver_run<const GIRTH:usize>(mut context: SteadyContext
                                   , rx: SteadyStreamRxBundle<StreamFragment, GIRTH>) -> Result<(), Box<dyn Error>> {
        let rx_meta = RxMetaDataHolder::new(rx.payload_meta_data());
        let mut cmd = into_monitor!(context, rx_meta, []);

        let mut rx = rx.lock().await;
        warn!("wait for two");//StreamRxBundle
        await_for_all!(cmd.wait_closed_or_avail_units_stream::<StreamFragment>(&mut rx, 1,1)); //not waiting for closed.
        
        // rx.defragment(&mut cmd);//TODO: change to cmd call
        // warn!("finished wait");
        // 
        // let mut vec_results = rx.take_by_stream(7); //TODO: change to cmd call
        // warn!("msg count: {}",vec_results.len());
        // warn!("first: {:?}",vec_results[0].data);
        // 
        // 
        // let data0 = vec_results[0].data.take().expect("");
        // assert_eq!([1,2,3], &data0[..]);
        // 
        // warn!("second: {:?}",vec_results[1].data);
        // 
        // let data1 = vec_results[1].data.take().expect("");
        // assert_eq!([4,5,6], &data1[..]);

        cmd.request_graph_stop();
        Ok(())
    }

    #[async_std::test]
    async fn test_bytes_process() {
        //TODO: these must be public exports or clone is broken.
        //TODO: these must be public exports or clone is broken.
        use crate::distributed::steady_stream::{LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundleClone, StreamFragment, StreamMessage};

        let mut graph = GraphBuilder::for_testing()
            .with_telemetry_metric_features(false)
            .build(());

        if !graph.is_aeron_media_driver_present() {
            info!("aeron test skipped, no media driver present");
            return;
        }

        let channel_builder = graph.channel_builder();

        let (to_aeron_tx,to_aeron_rx) = channel_builder.build_as_stream::<StreamMessage,1>(7);
        let (from_aeron_tx,from_aeron_rx) = channel_builder.build_as_stream::<StreamFragment,1>(7);

        let distribution = DistributionBuilder::aeron()
            .with_media_type(MediaType::Udp)
            .point_to_point(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
                            , 40123)
            .build();

        graph.build_stream_collector(distribution.clone()
                                     , "ReceiverTest"
                                     , from_aeron_tx
                                     , &mut Threading::Spawn);

        graph.build_stream_distributor(distribution.clone()
                                       , "SenderTest"
                                       , to_aeron_rx
                                       , &mut Threading::Spawn);
        // 
        // let x = to_aeron_tx.clone();
        // graph.actor_builder().with_name("MockSender")
        //     .build(move |context| mock_sender_run(context, to_aeron_tx.clone())
        //            , &mut Threading::Spawn);
        // 
        // graph.actor_builder().with_name("MockReceiver")
        //     .build(move |context| mock_receiver_run(context, from_aeron_rx.clone())
        //            , &mut Threading::Spawn);
        // 
        // graph.start(); //startup the graph
        // 
        // graph.block_until_stopped(Duration::from_secs(2));


    }

}
