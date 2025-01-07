
use std::error::Error;
use std::slice;
use num_traits::Zero;
use steady_state_aeron::aeron::*;
use steady_state_aeron::concurrent::logbuffer::frame_descriptor;
use crate::*;
use crate::{ into_monitor, steady_state, SteadyCommander, SteadyContext, SteadyState};
use crate::distributed::aeron_channel::Channel;
use crate::distributed::aqueduct::{AquaductTxDef, AquaductTxMetaData, AqueductFragment, FragmentDirection, FragmentType, SteadyAqueductTx};

#[derive(Default)]
pub(crate) struct FragmentState {
    arrival: Option<Instant>,
    sub_reg_id: Option<i64>,
}

/// In as much as possible this actor takes fragments and attempts to make them all contiguous in the
/// outgoing AqueductTx. For short Aqueducts and very large messages this will still result in fragments
/// but also a warning. This way the system keeps running but slower wit a warning to fix it by making
/// the messages smaller OR making the channel lager.
pub async fn run(context: SteadyContext
                 , tx: SteadyAqueductTx
                 , aeron_connect: Channel
                 , aeron: Arc<Mutex<Aeron>>
                 , state: SteadyState<FragmentState>) -> Result<(), Box<dyn Error>> {

    let md:AquaductTxMetaData = tx.meta_data();
    internal_behavior(into_monitor!(context, [], [md.control,md.payload]), tx, aeron_connect, aeron, state).await

}

/*
Fragment management:  - never combine, this is always done on the reader side!!
    We have 3 solutions to deal with the publishers which may have their frame fragments
    interleaved. These are the choices:
    (Default, fragmented)
    1. pass the fragments with header identifiers down the aqueduct and leave this
        up to the deserialization logic. Very complex on that end. May be useful for lowest
        latency and where we can consume partial messages.
    (Jumbo fragments)
    3. using N aqueuducts we can combine all fragments from the same publisher into
       a single frame. Working left to right we try to use the first aqueduct but only
       move to the next when we can combine to the current. The Headers have sequence
       numbers added to deserilize can still read them in order.  If we run out aqueducts
       then we still generate a fragment for the other end to deal with. That end might
       just hold a map of tossed frames so if the rest shows up later it can be
       reassembled or tossed.
 */


async fn internal_behavior<CMD: SteadyCommander>(mut cmd: CMD
                                                     , tx: SteadyAqueductTx
                                                     , aeron_channel: Channel
                                                     , aeron:Arc<Mutex<Aeron>>
                                                     , state: SteadyState<FragmentState>) -> Result<(), Box<dyn Error>> {
    let start = Instant::now();

    let mut state_guard = steady_state(&state, FragmentState::default).await;
    if let Some(state) = state_guard.as_mut() {

            use steady_state_aeron::concurrent::atomic_buffer::AtomicBuffer;
            use steady_state_aeron::concurrent::logbuffer::header::Header;
            use steady_state_aeron::image::ControlledPollAction;
            use steady_state_aeron::utils::types::Index;

            // trace!("Receiver register publication: {:?} {:?}",aeron_channel.cstring(), stream_id);
            let mut tx_lock = tx.lock().await;

            if state.sub_reg_id.is_none() {

                 //TODO: al the streams
                let stream_id = tx_lock.stream_first;

                //only add publication once if not already set
                    let reg = {
                        let mut locked_aeron = aeron.lock().await;
                        locked_aeron.add_subscription(aeron_channel.cstring(), stream_id)
                    };
                    match reg {
                        Ok(reg_id) => { state.sub_reg_id = Some(reg_id); }
                        Err(e) => {
                            warn!("Unable to add publication: {:?}, Check if Media Driver is running.",e);
                            return Err(e.into());
                        }
                    };
               
            }

            let subscription = match state.sub_reg_id {
                Some(a) => {
                 
                        //trace!("Looking for subscription {}",a);
                        loop {
                            let sub = {
                                let mut locked_aeron = aeron.lock().await;
                                locked_aeron.find_subscription(a)
                            };
                            match sub {
                                Ok(sub) => {
                                    //warn!("Found subscription!");
                                    break sub
                                },
                                Err(e) => {
                                    if e.to_string().contains("Awaiting") || e.to_string().contains("not ready") {
                                        yield_now::yield_now().await; //ok  we can retry again but should yeild until we can get the publication
                                        if cmd.is_liveliness_stop_requested() {
                                            warn!("stop detected before finiding publication");
                                            return Ok(()); //we are done, shutdown happened before we could start up.
                                        }
                                        //warn!("Try again finding publication: {:?}", e); //Error finding publication: Generic(ExclusivePublicationNotReadyYet { status: Awaiting })
                                    } else {
                                        warn!("Error finding publication: {:?}", e); //Error finding publication: Generic(ExclusivePublicationNotReadyYet { status: Awaiting })
                                        return Err(e.into());
                                    }
                                }
                            }
                            
                        }
                  
                }
                None => {
                    return Err("subscription registration id not available, check media driver".into());
                }
            };
            let mut subscription = match subscription.lock() {
                 Ok(s) => { s }
                 Err(e) => { return Err(format!("{:?}", &e).into()); }
            };

            let duration = start.elapsed();
            warn!("Receiver connected in: {:?} {:?} {:?}", duration, subscription.channel(), subscription.stream_id());
            while cmd.is_running(&mut || tx_lock.mark_closed()) {

                
                
                
                await_for_all!(cmd.wait_periodic(Duration::from_micros(20)),
                               cmd.wait_shutdown_or_vacant_units(&mut tx_lock.control_channel, 1)
                
                );

                //TODO: we need to avoid poill if we have no images!!
                // let channel_status = subscription.channel_status();
                // warn!(
                //         "Subscription channel status {}: {} ",
                //         channel_status,
                //         steady_state_aeron::concurrent::status::status_indicator_reader::channel_status_to_str(channel_status)
                //     );

                //NOTE: this is called once on every Image starting with a different Image each time around
                subscription.controlled_poll( &mut |buffer: &AtomicBuffer, offset: Index, length: Index, header: &Header| {
                    //NOTE: index is i32 so the current implementation limits us to 2GB messages !!
                    if state.arrival.is_none() {
                        state.arrival = Some(Instant::now());
                    }

                    let ctrl_room = cmd.vacant_units(&mut tx_lock.control_channel);
                    let room = cmd.vacant_units(&mut tx_lock.payload_channel);
                    let abort = room < length as usize && ctrl_room > 1;
                    if abort { //back off until we have room.
                        warn!("aborting due to lack of room: {} < {}, try again later", room, length);
                        Ok(ControlledPollAction::ABORT)
                    } else {

                        //trace!("we have room: {} >= {}", room, length);
                        let incoming_slice: &[u8] = unsafe {
                            slice::from_raw_parts_mut(buffer.buffer().offset(offset as isize)
                                                      , length as usize)
                        };
                        // we already checked so this better work
                        let bytes_consumed = cmd.send_slice_until_full(&mut tx_lock.payload_channel
                                                                       , incoming_slice);
                        debug_assert_eq!(bytes_consumed, length as usize);

                        let is_begin:bool = !(header.flags() & frame_descriptor::BEGIN_FRAG).is_zero();
                        let is_end:bool   = !(header.flags() & frame_descriptor::END_FRAG).is_zero();
                        let frame_type: FragmentType = if is_begin {
                                                       if is_end {
                                                           FragmentType::UnFragmented
                                                       } else {
                                                           FragmentType::Begin
                                                       }
                                                    } else {
                                                        if is_end {
                                                            FragmentType::End
                                                        } else {
                                                            FragmentType::Middle
                                                        }
                                                    };

                        //TODO: all streams
                        let stream_id = tx_lock.stream_first;

                        let ok = cmd.try_send(
                            &mut tx_lock.control_channel,
                            AqueductFragment::new(frame_type, stream_id, FragmentDirection::Incoming(header.session_id(), state.arrival.take().unwrap_or(Instant::now()) ), length)
                        );
                        debug_assert!(ok.is_ok()); //should not fail since we checked for room at the top before we started.

                        // Continue processing next fragments in this poll call.
                        Ok(ControlledPollAction::CONTINUE)
                    }
                }, 1 as i32);
            }
        Ok(())
    } else {
        Err("Failed to get state guard".into())
    }
}


#[cfg(test)]
pub(crate) mod aeron_media_driver_tests {
    use std::net::{IpAddr, Ipv4Addr};
    use futures_timer::Delay;
    use super::*;
    use crate::distributed::aeron_channel::{MediaType};
    use crate::distributed::distributed::{DistributionBuilder};

    #[async_std::test]
    async fn test_bytes_process() {

        let mut graph = GraphBuilder::for_testing()
            .with_telemetry_metric_features(false)
            .build(());

        let channel_builder = graph.channel_builder();
        let streams_first = 1;
        let streams_count = 1;
        let (to_aeron_tx,to_aeron_rx) = channel_builder.build_aqueduct(3
                                                                       ,30
                                     ,streams_first, streams_count);
        let (from_aeron_tx,from_aeron_rx) = channel_builder.build_aqueduct(3
                                                                           ,30
                                      ,streams_first, streams_count);

        let distribution = DistributionBuilder::aeron()
            .with_media_type(MediaType::Udp)
            .point_to_point(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
                            , 40125)
            .build();


        graph.build_aqueduct_distributor(distribution.clone()
                                         , "SenderTest"
                                         , to_aeron_rx.clone()
                                         , &mut Threading::Spawn);

        graph.build_aqueduct_collector(distribution.clone()
                                       , "ReceiverTest"
                                       , from_aeron_tx.clone()
                                       , &mut Threading::Spawn); //must not be same thread


        to_aeron_tx.testing_send_frame(&[1,2,3,4,5]).await;
        to_aeron_tx.testing_send_frame(&[6,7,8,9,10]).await;
        to_aeron_tx.testing_close().await;

        graph.start(); //startup the graph

        //we must wait long enough for both actors to connect
        //not sure why this needs to take so long but 14 sec seems to be smallest
        Delay::new(Duration::from_secs(20)).await;

        graph.request_stop();
        //we wait up to the timeout for clean shutdown which is transmission of all the data
        graph.block_until_stopped(Duration::from_secs(16));

        let mut data = [0u8; 5];
        let result = from_aeron_rx.testing_take_frame(&mut data[0..5]).await;
        assert_eq!(5, result);
        assert_eq!([1,2,3,4,5], data);
        let result = from_aeron_rx.testing_take_frame(&mut data[0..5]).await;
        assert_eq!(5, result);
        assert_eq!([6,7,8,9,10], data);

    }
}

