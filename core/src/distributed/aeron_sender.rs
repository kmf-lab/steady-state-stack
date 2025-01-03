
use std::error::Error;
use futures_timer::Delay;
use ringbuf::traits::Consumer;
use steady_state_aeron::aeron::Aeron;
use crate::{into_monitor, steady_state, SteadyCommander, SteadyContext, SteadyRx, SteadyState};
use crate::distributed::aqueduct::{AquaductRxDef, AquaductRxMetaData, SteadyAqueductRx, SteadyAqueductTx};
use crate::*;
use crate::distributed::aeron_channel::Channel;
use crate::distributed::nanosec_util::precise_sleep;

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

        #[cfg(any(
            feature = "aeron_driver_systemd",
            feature = "aeron_driver_sidecar",
            feature = "aeron_driver_external"
        ))]
        {
            use steady_state_aeron::concurrent::atomic_buffer::*;
            use steady_state_aeron::utils::types::Index;;
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
            //wait for media driver to be ready, normally 2 seconds at best so we can sleep
            Delay::new(Duration::from_millis(200)).await;

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

                    let to_read = if let Some(aquaduct_meta_data) = cmd.take_async(&mut rx_lock.control_channel).await {
                        aquaduct_meta_data.bytes_count //should not be zero?
                    } else {
                        warn!("error??");
                        //should not happen unless unclean shutdown
                        0
                    };

                    if to_read > 0 { //only send if we actually have some bytes?

                        //our channel must be large enough for the message so we can assume its all ready to go
                        //this is all the data in the channel but we only want those bytes which
                        //belong to our message
                        let (mut a, mut b) = rx_lock.payload_channel.rx.as_mut_slices();
                        let a_len = to_read.min(a.len());
                        let remaining_read = to_read - a_len;

                        let to_send = if 0 == remaining_read {
                            warn!("wrap slice {}",a_len);
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

                        warn!("Sending {:?} bytes", to_send.as_slice());
                        loop {
                            match publication.offer_part(to_send, 0, to_send.capacity()) {
                                Ok(value) => {
                                    warn!("Published {}", value);
                                    break;
                                },
                                Err(aeron_error) => {
                                    warn!("Error publishing data: {:?}", aeron_error);
                                    yield_now::yield_now().await; //ok  we can retry again but should yeild until we can get the publication
                                    let timeout = std::time::Instant::now() + std::time::Duration::from_secs(15);
                                    while publication.is_connected() == false {
                                        warn!("Waiting for Aeron publication to connect... {:?}",publication.channel());
                                        let channel_status = publication.channel_status();
                                        warn!(
                                            "Publication channel status {}: {} ",
                                            channel_status,
                                            steady_state_aeron::concurrent::status::status_indicator_reader::channel_status_to_str(channel_status)
                                        );
                                        if Instant::now() > timeout {
                                            error!("Timed out waiting for Aeron publication to connect.");
                                            return Err("Publication failed to connect".into());
                                        }
                                        yield_now::yield_now().await;
                                    }
                                }
                            }
                        }
                        unsafe { rx_lock.payload_channel.rx.advance_read_index(to_read); }

                        let channel_status = publication.channel_status();
                        warn!(
                            "Publication channel status {}: {} ",
                            channel_status,
                            steady_state_aeron::concurrent::status::status_indicator_reader::channel_status_to_str(channel_status)
                        );
                    } else {
                        warn!("no data to send!!!!!!!!!!!!!!! ");
                    }
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
    use crate::distributed::*;
    use crate::distributed::aeron_channel::{Endpoint, MediaType};
    use crate::distributed::aeron_channel::aeron_utils::aeron_context;
    use crate::distributed::aqueduct::{LazyAqueduct, LazyAqueductRx, LazyAqueductTx};
    use crate::distributed::distributed::{Distributed, DistributionBuilder};

    #[async_std::test]
    async fn test_bytes_process() {

        let mut graph = GraphBuilder::for_testing()
                                 .with_telemetry_metric_features(false)
                                 .build(());

        let control_builder = graph.channel_builder().with_capacity(10);
        let payload_builder = graph.channel_builder().with_capacity(1000);

        //TODO: build Distribution_aqueduct?
        let (to_aeron_tx,to_aeron_rx) = graph.build_aqueduct(&control_builder, &payload_builder);
        let (from_aeron_tx,from_aeron_rx) = graph.build_aqueduct(&control_builder, &payload_builder);

        let distribution = DistributionBuilder::aeron() //TODO: better name??
            .with_media_type(MediaType::Udp)
            .point_to_point(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 40123)
            .build(1);
        
        //sender and receiver need to shake hands so they must run on separate threads, TODO: needd to detect and warn!
        graph.build_distributed_tx(distribution.clone() //TODO: build_distributed_tx name??
                                   , "SenderTest"
                                   , to_aeron_rx.clone()
                                   , &mut Threading::Spawn);
        
        graph.build_distributed_rx(distribution.clone() //TODO: build_distributed_rx name??
                                   , "ReceiverTest"
                                   , from_aeron_tx.clone()
                                   , &mut Threading::Spawn);

        
        // TODO: write new actor to send data in blocks of bytes
        
        // TODO: write new actor to recieve data in blocks of bytes.
        
        // Check thouput.
        
        graph.start(); //startup the graph

        to_aeron_tx.testing_send_frame(&[1,2,3]).await;
        to_aeron_tx.testing_send_frame(&[4,5,6]).await;
        to_aeron_tx.testing_close().await;

        //we must wait long enough for both actors to connect
        //not sure why this needs to take so long but 14 sec seems to be smallest
        Delay::new(Duration::from_secs(14)).await;

        graph.request_stop();
        //we wait up to the timeout for clean shutdown which is transmission of all the data
        graph.block_until_stopped(Duration::from_secs(16));

        let mut data = [0u8; 3];
        let result = from_aeron_rx.testing_take_frame(&mut data[0..3]).await;
        assert_eq!(3, result);
        assert_eq!([1,2,3], data);
        let result = from_aeron_rx.testing_take_frame(&mut data[0..3]).await;
        assert_eq!(3, result);
        assert_eq!([4,5,6], data);



    }

}
