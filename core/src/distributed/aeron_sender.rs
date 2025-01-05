
use std::error::Error;
use futures_timer::Delay;
use num_traits::Zero;
use ringbuf::consumer::Consumer;
use steady_state_aeron::aeron::Aeron;
use crate::{into_monitor, steady_state, SteadyCommander, SteadyContext, SteadyRx, SteadyState};
use crate::distributed::aqueduct::{AquaductRxDef, AquaductRxMetaData, FragmentType, SteadyAqueductRx, SteadyAqueductTx};
use crate::*;
use crate::distributed::aeron_channel::Channel;

#[derive(Default)]
pub(crate) struct FragmentState {
    pub(crate) pub_reg_id: Option<i64>,    
    pub(crate) total_bytes: u128,
}

pub async fn run(context: SteadyContext, rx: SteadyAqueductRx, aeron_connect: Channel, stream_id: i32
                 , aeron:Arc<Mutex<Aeron>>, state: SteadyState<FragmentState>) -> Result<(), Box<dyn Error>> {
    let md:AquaductRxMetaData = rx.meta_data();
    internal_behavior(into_monitor!(context, [md.control,md.payload], []), rx, aeron_connect, stream_id, aeron, state).await
}



pub async fn internal_behavior<CMD: SteadyCommander>(mut cmd: CMD
                                                     , rx: SteadyAqueductRx
                                                     , aeron_channel: Channel
                                                     , stream_id: i32
                                                     ,  mut aeron:Arc<Mutex<Aeron>>
                                                     , state: SteadyState<FragmentState>
  ) -> Result<(), Box<dyn Error>> {

    let start = Instant::now();

    let mut state_guard = steady_state(&state, || FragmentState::default()).await;
    if let Some(mut state) = state_guard.as_mut() {
      
            use steady_state_aeron::concurrent::atomic_buffer::*;
            use steady_state_aeron::utils::types::Index;
            if state.pub_reg_id.is_none() {
                //only add publication once if not already set
                //trace!("Sender register publication: {:?} {:?}",aeron_channel.cstring(), stream_id);
                let reg = {
                    let mut locked_aeron = aeron.lock().await;
                    locked_aeron.add_exclusive_publication(aeron_channel.cstring(), stream_id)
                };
                match reg {
                    Ok(reg_id) => {
                        state.pub_reg_id = Some(reg_id);
                    }
                    Err(e) => {
                        warn!("Unable to add publication: {:?}, Check if Media Driver is running.",e);
                        return Err(e.into());
                    }
                };
            }
            let mut rx_lock = rx.lock().await;

            let mut publication = match state.pub_reg_id {
                Some(p) => {
                    // trace!("Looking for publication {} {:?} {:?}",p, aeron_channel.cstring(), stream_id);
                    loop {
                        let publication = {
                            let mut locked_aeron = aeron.lock().await;
                            locked_aeron.find_exclusive_publication(p)
                        };
                        //NOTE: this creates an "image" so the receiver can read this "image" and start to read data
                         match publication { //NOTE: switch to  find_exclusive_publication when we can
                                Err(e) => {
                                   if e.to_string().contains("Awaiting") || e.to_string().contains("not ready") {
                                       yield_now::yield_now().await; //ok  we can retry again but should yeild until we can get the publication
                                       if cmd.is_liveliness_stop_requested() {
                                           //trace!("stop detected before finding publication");
                                           return Ok(()); //we are done, shutdown happened before we could start up.
                                       }
                                   } else {
                                       warn!("Error finding publication: {:?}", e);
                                       return Err(e.into());
                                   }
                                },
                                Ok(publ) => {
                                       //trace!("publication found");
                                       break publ
                                }
                           }
                    }
                },
                None => {
                    return Err("publication registration id not available, check media driver".into());
                }
            };
            let mut publication = publication.lock().expect("internal");

            let duration = start.elapsed();
            warn!("Sender connected to Aeron publication in {:?}", duration);
            //TODO: we should detect if we break out of is_running loop and report a warning

            while cmd.is_running(&mut || rx_lock.is_closed_and_empty()) {
                //warn!("waiting for rx message");
                let clean = await_for_all!(cmd.wait_closed_or_avail_units(&mut rx_lock.control_channel, 1));
                if clean {

                    let to_read = if let Some(aquaduct_frame) = cmd.take_async(&mut rx_lock.control_channel).await {
                        if aquaduct_frame.length.is_zero() {
                            warn!("actor sent zero length message to be sent, this should be avoided for performance reasons.");
                        }
                        
                        
                        let _ = aquaduct_frame.option;
                        //TODO: let _ = aqueduct_frame.message_id  should be optional for send !!
                        
                        
                        //TODO: in this early iteration we only support full unfragmented blocks for send
                        assert_eq!(aquaduct_frame.fragment_type, FragmentType::UnFragmented); 
                        aquaduct_frame.length
                        
                    } else {
                        0 //this case may happen during shutdown and we ignore it with if to_read>0
                    } as usize;

                    if to_read > 0 { //only send if we actually have some bytes?

                        //our channel must be large enough for the message so we can assume its all ready to go
                        //this is all the data in the channel but we only want those bytes which
                        //belong to our message
                        let (mut a, mut b) = rx_lock.payload_channel.rx.as_mut_slices();
                        let a_len = to_read.min(a.len());
                        let remaining_read = to_read - a_len;

                        let to_send = if 0 == remaining_read {
                            //trace!("wrap slice {}",a_len);
                            AtomicBuffer::wrap_slice(&mut a[0..a_len])
                        } else {
                            //rare: we are going over the edge so we are forced to copy the data
                            //NOTE: with patch to exclusive_publication we could avoid this copy
                            let aligned_buffer = AlignedBuffer::with_capacity(to_read as Index);
                            let mut buf = AtomicBuffer::from_aligned(&aligned_buffer);
                            buf.put_bytes(0, &mut a[0..a_len]);
                            let b_len = remaining_read.min(b.len());
                            let extended_read = remaining_read - b_len;
                            buf.put_bytes(a_len as Index, &mut b[0..b_len]);
                            assert_eq!(0, extended_read); //we should have read all the data
                            warn!("copy buffer {} {}",a_len,b_len);
                            buf
                        };

                        //trace!("Sending {:?} bytes", to_send.as_slice());
                        loop {
                            match publication.offer_part(to_send, 0, to_send.capacity()) {
                                Ok(value) => {
                                    warn!("Published {:?} {}", to_send.as_slice(), value);
                                    break;
                                },
                                Err(aeron_error) => {
                                    warn!("Error publishing data: {:?}", aeron_error);
                                    yield_now::yield_now().await; //ok  we can retry again but should yeild until we can get the publication
                                    let timeout = std::time::Instant::now() + std::time::Duration::from_secs(15);
                                    while publication.is_connected() == false {
                                        yield_now::yield_now().await;
                                        Delay::new(Duration::from_millis(20)).await;
                                        
                                        warn!("Waiting for Aeron publication to connect... {:?}",publication.channel());
                                        // let channel_status = publication.channel_status();
                                        // warn!(
                                        //     "Publication channel status {}: {} ",
                                        //     channel_status,
                                        //     steady_state_aeron::concurrent::status::status_indicator_reader::channel_status_to_str(channel_status)
                                        // );
                                        if Instant::now() > timeout {
                                            error!("Timed out waiting for Aeron publication to connect.");
                                            return Err("Publication failed to connect".into());
                                        }
                                    }
                                }
                            }
                        }
                        unsafe { rx_lock.payload_channel.rx.advance_read_index(to_read); }

                        // let channel_status = publication.channel_status();
                        // warn!(
                        //     "Publication channel status {}: {} ",
                        //     channel_status,
                        //     steady_state_aeron::concurrent::status::status_indicator_reader::channel_status_to_str(channel_status)
                        // );
                    } 
               }
           }
   
    Ok(())
} else {
Err("State not available".into())
    }
}


#[cfg(test)]
pub(crate) mod aeron_tests {
    use std::net::{IpAddr, Ipv4Addr};
    use futures_timer::Delay;
    use super::*;
    use crate::distributed::aeron_channel::{MediaType};
    use crate::distributed::aqueduct::AqueductFragment;
    use crate::distributed::distributed::{DistributionBuilder};

    //TODO: Send mb of data so we can time it and check (head tail neither both)
    //TODO: need sealize deserlize trate examples. (last if we have time)
    //TODO: need byte sending api first for all testing and release.
    //TODO: clean up the code
    
    pub async fn mock_sender_run(mut context: SteadyContext
                                 , tx: SteadyAqueductTx) -> Result<(), Box<dyn Error>> {

        //required to ensure we do not stop before everyinng is up..
        //context.is_running() //TODO: check the regirstion and vote logic.
        
        //TODO: AqueductFrame seems differnet in and out we must re-think this.
        
        let mut tx = tx.lock().await;
        let len = context.send_slice_until_full(&mut tx.payload_channel, &[1,2,3]);
        let _ = context.try_send(&mut tx.control_channel, AqueductFragment::simple_outgoing(0, 3));

        let len = context.send_slice_until_full(&mut tx.payload_channel, &[4,5,6]);
        let _ = context.try_send(&mut tx.control_channel, AqueductFragment::simple_outgoing(0, 3));

        //output we can only write a full block  beginnint to end alll byte count
        // we could write some bytes in different batchs but they al must fit in the channel or we cant send them.
        
        
        tx.payload_channel.mark_closed();
        tx.control_channel.mark_closed();

        Ok(())
    }

    pub async fn mock_receiver_run(mut context: SteadyContext
                                   , rx: SteadyAqueductRx) -> Result<(), Box<dyn Error>> {

        
        //reading from the network may be brokeninto parts.
        //  we may read head, body, tail, all ??  next big problem!!
        
        
        // read data into vec by stream and producer
        // check apis for copy or peek into vec operations.
        
        let mut rx = rx.lock().await;
        
        //need a method signature for this pattern
        rx.defragment(&mut context);
        let frames = rx.take_stream_arrival(1);
        
        
        await_for_all!(context.wait_closed_or_avail_units(&mut rx.control_channel, 2));

        let _ = context.try_take(&mut rx.control_channel);
        let mut data = [0u8; 3];
        let result = context.take_slice(&mut rx.payload_channel, &mut data[0..3]);
        //assert_eq!(3, result);
        //assert_eq!([1,2,3], data);

        let _ = context.try_take(&mut rx.control_channel);
        let result = context.take_slice(&mut rx.payload_channel, &mut data[0..3]);
        //assert_eq!(3, result);
        //assert_eq!([4,5,6], data);

        Ok(())
    }

    #[async_std::test]
    async fn test_bytes_process() {

        let mut graph = GraphBuilder::for_testing()
                                 .with_telemetry_metric_features(false)
                                 .build(());
 
        let channel_builder = graph.channel_builder();
        let (to_aeron_tx,to_aeron_rx) = channel_builder.build_aqueduct(10
                                                                       ,1000);
        let (from_aeron_tx,from_aeron_rx) = channel_builder.build_aqueduct(10
                                                                       ,1000);

        let distribution = DistributionBuilder::aeron()
            .with_media_type(MediaType::Udp)
            .point_to_point(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
                            , 40123)
            .build(7);//we must not collide with the other test
        
        graph.build_aqueduct_distributor(distribution.clone()
                                         , "SenderTest"
                                         , to_aeron_rx.clone()
                                         , &mut Threading::Spawn);
        
        graph.build_aqueduct_collector(distribution.clone()
                                       , "ReceiverTest"
                                       , from_aeron_tx.clone()
                                       , &mut Threading::Spawn); //must not be same thread

        graph.actor_builder().with_name("MockSender")            
                             .build(move |context| mock_sender_run(context, to_aeron_tx.clone())
                            , &mut Threading::Spawn);

        graph.actor_builder().with_name("MockReceiver")
                             .build(move |context| mock_receiver_run(context, from_aeron_rx.clone())
                            , &mut Threading::Spawn);

        graph.start(); //startup the graph

        //we must wait long enough for both actors to connect
        //not sure why this needs to take so long but 14 sec seems to be smallest
        Delay::new(Duration::from_secs(22)).await;

        graph.request_stop();
        //we wait up to the timeout for clean shutdown which is transmission of all the data
        graph.block_until_stopped(Duration::from_secs(16));

   

    }

}
