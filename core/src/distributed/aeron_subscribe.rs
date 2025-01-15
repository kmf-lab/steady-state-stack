use std::error::Error;
use std::slice;
use std::sync::{Arc};
use futures_timer::Delay;
use steady_state_aeron::aeron::Aeron;
use steady_state_aeron::concurrent::atomic_buffer::AtomicBuffer;
use steady_state_aeron::concurrent::logbuffer::frame_descriptor;
use steady_state_aeron::concurrent::logbuffer::header::Header;
use steady_state_aeron::image::ControlledPollAction;
use steady_state_aeron::subscription::Subscription;
use crate::distributed::aeron_channel::Channel;
use crate::distributed::steady_stream::{SteadyStreamTxBundle, SteadyStreamTxBundleTrait, StreamTxBundleTrait, StreamFragment, FragmentType};
use crate::{into_monitor, SteadyCommander, SteadyContext, SteadyState};
use crate::*;
use crate::monitor::{TxMetaDataHolder};

#[derive(Default)]
pub(crate) struct AeronSubscribeSteadyState {
    arrival: Option<Instant>,
    sub_reg_id: Vec<Option<i64>>,
}

pub async fn run<const GIRTH:usize,>(context: SteadyContext
                                     , tx: SteadyStreamTxBundle<StreamFragment,GIRTH>
                                     , aeron_connect: Channel
                                     , aeron:Arc<futures_util::lock::Mutex<Aeron>>
                                     , state: SteadyState<AeronSubscribeSteadyState>) -> Result<(), Box<dyn Error>> {
    let tx_meta = TxMetaDataHolder::new(tx.payload_meta_data());
    internal_behavior(into_monitor!(context, [], tx_meta), tx, aeron_connect, aeron, state).await
}

async fn internal_behavior<const GIRTH:usize,C: SteadyCommander>(mut cmd: C
                                                                 , tx: SteadyStreamTxBundle<StreamFragment,GIRTH>
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
            warn!("holding add_subscription lock");
            for f in 0..GIRTH {
                if state.sub_reg_id[f].is_none() { //only add if we have not already done this
                    warn!("adding new sub {} ", tx[f].stream_id);
                    match aeron.add_subscription(aeron_channel.cstring(), tx[f].stream_id) {
                        Ok(reg_id) => state.sub_reg_id[f] = Some(reg_id),
                        Err(e) => {
                            warn!("Unable to add publication: {:?}",e);
                        }
                    };
                }
            };
            warn!("released add_subscription lock");
        }
        Delay::new(Duration::from_millis(2)).await; //back off so our request can get ready

    // now lookup when the subscriptions are ready
        for f in 0..GIRTH {
            if let Some(id) = state.sub_reg_id[f] {
                subs[f] = loop {
                    let sub = {
                        let mut aeron = aeron.lock().await; //caution other actors need this so do jit
                        warn!("holding find_subscription({}) lock",id);
                        let response = aeron.find_subscription(id);
                        warn!("released find_subscription({}) lock",id);
                        response
                    };
                    match sub {
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

        warn!("running subscriber");
        while cmd.is_running(&mut || tx.mark_closed()) {
     
            // only poll this often
            await_for_all!( cmd.wait_periodic(Duration::from_millis(10)) );

            for i in 0..GIRTH {
                let max_poll_frags = tx[i].control_channel.vacant_units();
                if max_poll_frags>0 {
                    match &mut subs[i] {
                        Ok(ref mut sub) => {
                         //   warn!("called poll");

                            sub.controlled_poll(&mut |buffer: &AtomicBuffer
                                                      , offset: i32
                                                      , length: i32
                                                      , header: &Header| {
                                //NOTE: index is i32 so the current implementation limits us to 2GB messages !!
                                if state.arrival.is_none() {
                                    state.arrival = Some(Instant::now());
                                }
                                //trace!("poll found something");

                                let ctrl_room = cmd.vacant_units(&mut tx[i].control_channel);
                                let room = cmd.vacant_units(&mut tx[i].payload_channel);
                                let abort = room < length as usize && ctrl_room > 1;
                                if abort { //back off until we have room.
                                    //move on to the next of the bundle
                                    //TODO: in the future return quicker to this one since we KNOW we have data
                                    warn!("postpone due to lack of room: {} < {}, try again later", room, length);
                                    Ok(ControlledPollAction::ABORT)
                                } else {
                                    warn!("poll we have Room:{} >= Len:{} frameLen:{:?}", room, length, header.frame_length());
                                    let incoming_slice: &[u8] = unsafe {
                                        slice::from_raw_parts_mut(buffer.buffer().offset(offset as isize)
                                                                  , length as usize)
                                    };
                                    let bytes_consumed = cmd.send_slice_until_full(&mut tx[i].payload_channel
                                                                                   , incoming_slice);
                                    debug_assert_eq!(bytes_consumed, length as usize);

                                    let is_begin: bool = 0 != (header.flags() & frame_descriptor::BEGIN_FRAG);
                                    let is_end: bool = 0 != (header.flags() & frame_descriptor::END_FRAG);
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

                                    let ok = cmd.try_send(&mut tx[i].control_channel,
                                                          StreamFragment {
                                                              length,
                                                              session_id: header.session_id(),
                                                              arrival: Instant::now(),
                                                              fragment_type: frame_type,
                                                          }
                                    );
                                    debug_assert!(ok.is_ok()); //should not fail since we checked for room at the top before we started.

                                    // Continue processing next fragments in this poll call.
                                    Ok(ControlledPollAction::CONTINUE)
                                }
                            }, max_poll_frags as i32);
                        }
                        Err(e) => {
                            panic!("{:?}",e); //restart this actor for some reason we do not have subscription
                        }
                    }
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
    use crate::distributed::aeron_channel::MediaType;
    use crate::distributed::aeron_distributed::DistributionBuilder;
    use crate::distributed::steady_stream::StreamMessage;

    #[async_std::test]
    async fn test_bytes_process() {
        let mut graph = GraphBuilder::for_testing()
            .with_telemetry_metric_features(false)
            .build(());

        if !graph.is_aeron_media_driver_present() {
            info!("aeron test skipped, no media driver present");
            return;
        }

        let channel_builder = graph.channel_builder();

        //TODO: need a macro to make this easier?

        let (to_aeron_tx,to_aeron_rx) = channel_builder.build_as_stream::<StreamMessage,2>(0);
        let (from_aeron_tx,from_aeron_rx) = channel_builder.build_as_stream::<StreamFragment,2>(0);

        let distribution = DistributionBuilder::aeron()
            .with_media_type(MediaType::Udp)
            .point_to_point(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
                            , 40125)
            .build();

        graph.build_stream_collector(distribution.clone()
                                     , "ReceiverTest"
                                     , from_aeron_tx
                                     , &mut Threading::Spawn);

        graph.build_stream_distributor(distribution.clone()
                                         , "SenderTest"
                                         , to_aeron_rx
                                         , &mut Threading::Spawn);


        to_aeron_tx[0].testing_send_frame(&[1,2,3,4,5]).await;
        to_aeron_tx[0].testing_send_frame(&[6,7,8,9,10]).await;
        to_aeron_tx[0].testing_close().await;
        to_aeron_tx[1].testing_close().await; //TODO: shutdown should help with why we have a no vote and provide what we are waiting on??

        graph.start(); //startup the graph

        //wait till we see 2 full fragments back
        from_aeron_rx[0].testing_avail_wait(2).await;

        graph.request_stop();
        //we wait up to the timeout for clean shutdown which is transmission of all the data
        graph.block_until_stopped(Duration::from_secs(4));

        //from_aeron_rx.

        let mut data = [0u8; 5];
        let result = from_aeron_rx[0].testing_take_frame(&mut data[0..5]).await;
        assert_eq!(5, result);
        assert_eq!([1,2,3,4,5], data);
        let result = from_aeron_rx[0].testing_take_frame(&mut data[0..5]).await;
        assert_eq!(5, result);
        assert_eq!([6,7,8,9,10], data);

    }
}


