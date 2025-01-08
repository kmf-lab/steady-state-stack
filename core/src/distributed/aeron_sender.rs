
use std::error::Error;
use futures_timer::Delay;
use num_traits::Zero;
use ringbuf::consumer::Consumer;
use steady_state_aeron::aeron::Aeron;
use steady_state_aeron::concurrent::logbuffer::term_appender::OnReservedValueSupplier;
use steady_state_aeron::exclusive_publication::ExclusivePublication;
use crate::{into_monitor, steady_state, SteadyCommander, SteadyContext, SteadyState};
use crate::distributed::aqueduct::{AquaductRxDef, AquaductRxMetaData, FragmentType, SteadyAqueductRx};
use crate::*;
use crate::distributed::aeron_channel::Channel;

#[derive(Default)]
pub(crate) struct FragmentState {
    pub(crate) pub_reg_id: Vec<i64>,
}

/// Aeron Sender of messages taken from SteadyAqueduct
pub async fn run(context: SteadyContext
                 , rx: SteadyAqueductRx
                 , aeron_connect: Channel
                 , aeron:Arc<Mutex<Aeron>>
                 , state: SteadyState<FragmentState>) -> Result<(), Box<dyn Error>> {
    let md:AquaductRxMetaData = rx.meta_data();
    internal_behavior(into_monitor!(context, [md.control,md.payload], []), rx, aeron_connect, aeron, state).await
}


async fn internal_behavior<CMD: SteadyCommander>(mut cmd: CMD
                                                     , rx: SteadyAqueductRx
                                                     , aeron_channel: Channel
                                                     , mut aeron:Arc<Mutex<Aeron>>
                                                     , state: SteadyState<FragmentState>
  ) -> Result<(), Box<dyn Error>> {

    let start = Instant::now();

    let mut state_guard = steady_state(&state, || FragmentState::default()).await;
    if let Some(state) = state_guard.as_mut() {
            use steady_state_aeron::concurrent::atomic_buffer::*;
            use steady_state_aeron::utils::types::Index;

            let mut rx_lock = rx.lock().await;

            for i in 0..rx_lock.stream_count { //on actor restart we will use existing ids
                if state.pub_reg_id.len()== i as usize {    //only add publication once if not already set
                    let stream_id = rx_lock.stream_first+i;
                    //trace!("Sender register publication: {:?} {:?}",aeron_channel.cstring(), stream_id);
                    match aeron.lock().await.add_exclusive_publication(aeron_channel.cstring(), stream_id) {
                        Ok(reg_id) => state.pub_reg_id.push(reg_id),
                        Err(e) => {
                            warn!("Unable to add publication: {:?}, Check if Media Driver is running.",e);
                            return Err(e.into());
                        }
                    };
                }
            }
            //all publications have been registered giving the media driver lots to do

        let mut publication_vec: Vec<ExclusivePublication> = Vec::with_capacity(state.pub_reg_id.len());

        for id in state.pub_reg_id.iter() {
            let (publ, result) = lookup_publication(&mut cmd, &mut aeron, *id).await;
            if let Some(p) = publ {
                // Take ownership of the Arc and unwrap it
                match Arc::try_unwrap(p) {
                    Ok(mutex) => {
                        // Take ownership of the inner Mutex
                        match mutex.into_inner() {
                            Ok(publication) => {
                                // Successfully extracted the ExclusivePublication
                                publication_vec.push(publication);
                            }
                            Err(_) => panic!("Failed to unwrap Mutex"),
                        }
                    }
                    Err(_) => panic!("Failed to unwrap Arc. Are there other references?"),
                }
            } else {
                return result;
            }
        }

        // Now `publication_vec` holds all the `ExclusivePublication` instances, unwrapped from Arc and Mutex.
     

        let duration = start.elapsed();
        warn!("Sender connected to Aeron publication(s) in {:?}", duration);

            while cmd.is_running(&mut || rx_lock.is_closed_and_empty()) {
                //warn!("waiting for rx message");
                let clean = await_for_all!(cmd.wait_closed_or_avail_units(&mut rx_lock.control_channel, 1));
                if clean {
                    let (stream_index,to_read) = if let Some(aquaduct_frame) = cmd.take_async(&mut rx_lock.control_channel).await {
                        if aquaduct_frame.length.is_zero() {
                            warn!("actor sent zero length message to be sent, this should be avoided for performance reasons.");
                        }
                        //TODO: in this early iteration we only support full unfragmented blocks for send
                        assert_eq!(aquaduct_frame.fragment_type, FragmentType::UnFragmented);

                        debug_assert!(aquaduct_frame.stream_id >= rx_lock.stream_first);
                        debug_assert!(aquaduct_frame.stream_id < rx_lock.stream_first+rx_lock.stream_count);
                        //trace!("subtraction {} - {} ", aquaduct_frame.stream_id, rx_lock.stream_first);

                        ((aquaduct_frame.stream_id - rx_lock.stream_first) as usize,aquaduct_frame.length as usize)
                    } else {
                        (usize::MAX,0) //this case may happen during shutdown and we ignore it with if to_read>0
                    };

                    if to_read > 0 { //only send if we actually have some bytes?

                        //our channel must be large enough for the message so we can assume its all ready to go
                        //this is all the data in the channel but we only want those bytes which
                        //belong to our message
                        let (a, b) = rx_lock.payload_channel.rx.as_mut_slices();
                        let a_len = to_read.min(a.len());
                        let remaining_read = to_read - a_len;

                        let to_send = if 0 == remaining_read {
                            //trace!("wrap slice {}",a_len);
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
                            warn!("copy buffer {} {}",a_len,b_len);
                            buf
                        };

                        //trace!("Sending {:?} bytes", to_send.as_slice());
                        loop {
                            const APP_ID:i64 = 8675309;
                            //TODO: need to use this field to hash payload with our private key.
                            let f:OnReservedValueSupplier = |_buf,_start,_len| {APP_ID};
                            //TODO: in the reader if this value does not match expected set the Result Err so we know 
                            warn!("index {} pubs {} stream_first {}",stream_index, publication_vec.len(), rx_lock.stream_first );
                            let offer_response = publication_vec[stream_index].offer_opt(to_send, 0, to_send.capacity(), f);

                            match offer_response {
                                Ok(value) => {
                                    warn!("Published {:?} {}", to_send.as_slice(), value);
                                    break;
                                },
                                Err(aeron_error) => {
                                    warn!("Error publishing data: {:?}", aeron_error);
                                    yield_now::yield_now().await; //ok  we can retry again but should yeild until we can get the publication
                                    let timeout = Instant::now() + Duration::from_secs(15);
                                    while !publication_vec[stream_index].is_connected() {
                                        
                                        //trace!("Waiting for Aeron publication to connect... {:?}",publication[stream_index].channel());
                                        
                                        // let channel_status = publication.channel_status();
                                        // warn!(
                                        //     "Publication channel status {}: {} ",
                                        //     channel_status,
                                        //     steady_state_aeron::concurrent::status::status_indicator_reader::channel_status_to_str(channel_status)
                                        // );
                                        cmd.wait_periodic(Duration::from_millis(40)).await;
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

async fn lookup_publication<CMD: SteadyCommander>(cmd: &mut CMD, aeron: &mut Arc<Mutex<Aeron>>, id: i64) ->
         (Option< Arc<std::sync::Mutex<ExclusivePublication>> >, Result<(), Box<dyn Error>>) {
    loop {
        //NOTE: this creates an "image" so the receiver can read this "image" and start to read data
        match aeron.lock().await.find_exclusive_publication(id) { //NOTE: switch to  find_exclusive_publication when we can
            Err(e) => {
                if e.to_string().contains("Awaiting") || e.to_string().contains("not ready") {
                    yield_now::yield_now().await; //ok  we can retry again but should yeild until we can get the publication
                    if cmd.is_liveliness_stop_requested() {
                        //trace!("stop detected before finding publication");
                        //we are done, shutdown happened before we could start up.
                        break (None,Ok(()));
                    }
                } else {
                    warn!("Error finding publication: {:?}", e);
                    break (None,Err(e.into()));
                }
            },
            Ok(publication) => {
                break (Some(publication),Ok(()));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod aeron_tests {
    use std::net::{IpAddr, Ipv4Addr};
    use futures_timer::Delay;
    use super::*;
    use crate::distributed::aeron_channel::{MediaType};
    use crate::distributed::aqueduct::{AquaductTxDef, AquaductTxMetaData, AqueductFragment, SteadyAqueductTx};
    use crate::distributed::distributed::{DistributionBuilder};

    pub async fn mock_sender_run(mut context: SteadyContext
                                 , tx: SteadyAqueductTx) -> Result<(), Box<dyn Error>> {

        let md:AquaductTxMetaData = tx.meta_data();
        let mut cmd = into_monitor!(context, [], [md.control,md.payload]);

        let mut tx = tx.lock().await;
        //normally would be a while but for this test we only need to send these two.
        //TODO: send MB of data..
        if cmd.is_running(&mut || tx.mark_closed()) {

            let stream_id = 7;
            await_for_all!(cmd.wait_shutdown_or_vacant_aqueduct(&mut tx, 2, 1000));
            cmd.try_aqueduct_send(&mut tx, stream_id, &[1,2,3]);
            cmd.try_aqueduct_send(&mut tx, stream_id, &[4,5,6]);
        }

        tx.mark_closed();

        Ok(())
    }

    pub async fn mock_receiver_run(mut context: SteadyContext
                                   , rx: SteadyAqueductRx) -> Result<(), Box<dyn Error>> {

        let md:AquaductRxMetaData = rx.meta_data();
        let mut cmd = into_monitor!(context, [md.control,md.payload], []);

        let mut rx = rx.lock().await;
        await_for_all!(cmd.wait_avail_units(&mut rx.control_channel, 2)); //not waiting for closed.

        //TODO: this test must be skippe in github without aeron..
        rx.defragment(&mut cmd);//TODO: change to cmd call
        let mut vec_results = rx.take_by_stream(7); //TODO: change to cmd call
        warn!("length: {}",vec_results.len());
        warn!("first: {:?}",vec_results[0].data);


        let data0 = vec_results[0].data.take().expect("");
        let data1 = vec_results[1].data.take().expect("");

        assert_eq!([1,2,3], &data0[..]);
        assert_eq!([4,5,6], &data1[..]);

        Ok(())
    }

    #[async_std::test]
    async fn test_bytes_process() {

        let mut graph = GraphBuilder::for_testing()
                                 .with_telemetry_metric_features(false)
                                 .build(());
 
        let channel_builder = graph.channel_builder();
        let streams_first = 7;
        let streams_count = 1;
        let (to_aeron_tx,to_aeron_rx) = channel_builder.build_aqueduct(10
                                                                       ,1000
                                                        ,streams_first, streams_count);
        let (from_aeron_tx,from_aeron_rx) = channel_builder.build_aqueduct(10
                                                                       ,1000
                                                        ,streams_first, streams_count);

        let distribution = DistributionBuilder::aeron()
            .with_media_type(MediaType::Udp)
            .point_to_point(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
                            , 40123)
            .build();

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
