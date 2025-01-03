
use std::error::Error;
use std::slice;
use futures_timer::Delay;
use steady_state_aeron::aeron::Aeron;
use crate::*;
use crate::{await_for_all, into_monitor, steady_state, SendSaturation, SteadyCommander, SteadyContext, SteadyRxBundle, SteadyState, SteadyTx, SteadyTxBundle};
use crate::distributed::aeron_channel::Channel;
use crate::distributed::aqueduct::{AquaductRxMetaData, AquaductTxDef, AquaductTxMetaData, AqueductMetadata, SteadyAqueductTx};

#[derive(Default)]
pub(crate) struct FragmentState {
    pub(crate) total_bytes: u128,
    pub(crate) sub_reg_id: Option<i64>,
}

/// In as much as possible this actor takes fragments and attempts to make them all contiguous in the
/// outgoing AqueductTx. For short Aqueducts and very large messages this will still result in fragments
/// but also a warning. This way the system keeps running but slower wit a warning to fix it by making
/// the messages smaller OR making the channel lager.
pub async fn run(context: SteadyContext
                 , tx: SteadyAqueductTx
                 , aeron_connect: Channel
                 , stream_id: i32
                 , aeron: Arc<Mutex<Aeron>>
                 , state: SteadyState<FragmentState>) -> Result<(), Box<dyn Error>> {

    let md:AquaductTxMetaData = tx.meta_data();
    internal_behavior(into_monitor!(context, [], [md.control,md.payload]), tx, aeron_connect, stream_id, aeron, state).await

}

/*
Fragment management:
    We have 3 solutions to deal with the publishers which may have their frame fragments
    interleaved. These are the choices:
    (Default, fragmented)
    1. pass the fragments with header identifiers down the aqueduct and leave this
        up to the deserialization logic. Very complex on that end. May be useful for lowest
        latency and where we can consume partial messages.
    (Jumbo fragments)
    2. when possible combine the current fragment with the next when they are from the
       same publisher then generate new header. This does not eliminate fragments but
       reduces them. This takes no extra memory and is not difficult to implement.
    (Minimize fragments)
    3. using N aqueuducts we can combine all fragments from the same publisher into
       a single frame. Working left to right we try to use the first aqueduct but only
       move to the next when we can combine to the current. The Headers have sequence
       numbers added to deserilize can still read them in order.  If we run out aqueducts
       then we still generate a fragment for the other end to deal with. That end might
       just hold a map of tossed frames so if the rest shows up later it can be
       reassembled or tossed.
 */


pub async fn internal_behavior<CMD: SteadyCommander>(mut cmd: CMD
                                                     , tx: SteadyAqueductTx
                                                     , aeron_channel: Channel
                                                     , stream_id: i32
                                                     , mut aeron:Arc<Mutex<Aeron>>
                                                     , state: SteadyState<FragmentState>) -> Result<(), Box<dyn Error>> {
    let start = Instant::now();

    let mut state_guard = steady_state(&state, || FragmentState::default()).await;
    if let Some(mut state) = state_guard.as_mut() {

        #[cfg(any(
            feature = "aeron_driver_systemd",
            feature = "aeron_driver_sidecar",
            feature = "aeron_driver_external"
        ))]
        {
            use steady_state_aeron::concurrent::atomic_buffer::AtomicBuffer;
            use steady_state_aeron::concurrent::logbuffer::header::Header;
            use steady_state_aeron::image::ControlledPollAction;
            use steady_state_aeron::utils::types::Index;
            use crate::distributed::nanosec_util::precise_sleep;
            use crate::distributed::aeron_channel::aeron_utils::aeron_context;

           // Delay::new(Duration::from_secs(3)).await; //LET the sender start up first.


            warn!("Receiver register publication: {:?} {:?}",aeron_channel.cstring(), stream_id);

            if state.sub_reg_id.is_none() {
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
            let mut tx_lock = tx.lock().await;
            //wait for media driver to be ready, normally 2 seconds at best so we can sleep
            Delay::new(Duration::from_millis(200)).await; 
            
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
                //TODO: only if we are using nano-ticks
                //await_for_all!(cmd.wait_shutdown_or_vacant_units(&mut tx_lock.control_channel, 1));

                //TODO: only if we are not using nano-ticks
                await_for_all!(cmd.wait_periodic(Duration::from_micros(100)),
                               cmd.wait_shutdown_or_vacant_units(&mut tx_lock.control_channel, 1));



                //TODO: we need to avoid poill if we have no images!!
                // let channel_status = subscription.channel_status();
                // warn!(
                //         "Subscription channel status {}: {} ",
                //         channel_status,
                //         steady_state_aeron::concurrent::status::status_indicator_reader::channel_status_to_str(channel_status)
                //     );

                //NOTE: this is called once on every Image starting with a different Image each time around
                subscription.controlled_poll( &mut |buffer: &AtomicBuffer, offset: Index, length: Index, header: &Header| {
           
                    let room = cmd.vacant_units(&mut tx_lock.payload_channel);
                    let abort = room < length as usize;
                    if abort { //back off until we have room.
                        warn!("aborting due to lack of room: {} < {}", room, length);
                        Ok(ControlledPollAction::ABORT)
                    } else {
                        warn!("we have room: {} >= {}", room, length);
                        let incoming_slice: &[u8] = unsafe {
                            slice::from_raw_parts_mut(buffer.buffer().offset(offset as isize)
                                                      , length as usize)
                        };
                        // we already checked so this better work
                        let bytes_consumed = cmd.send_slice_until_full(&mut tx_lock.payload_channel
                                                                       , incoming_slice);
                        assert_eq!(bytes_consumed, length as usize);
                        assert_eq!(stream_id, header.stream_id());
                        
                        //for this image we need to hold state until we see the end and then drop the meta data
                        
                        //TODO: stop here and write the sender code to ensure:
                        // timestamp and use of stream id and session id.
                        
                        //assert this is expected to match
                        let _s = header.session_id(); // different publishers all on this stream
                        let _f = header.flags();       // end or beginning of data frame (mutiple per publisher)
                        //     header.reserved_value() // we could use this for time??
                        // we have no need to worry about Image since we have stream and session
                        // could we use reserved_value for timestamp to detect latencies?
                        warn!("Fragment: {} {} {} {} {} bytes: {}"
                              , header.stream_id()
                              , header.session_id()
                              , header.flags()
                              , header.term_id()
                              , header.term_offset()
                              , bytes_consumed);
                        
                        let ok = cmd.try_send(&mut tx_lock.control_channel, AqueductMetadata { bytes_count: bytes_consumed, bonus: 0 });
                        if let Err(e) = ok {
                            warn!("Error sending metadata: {:?}", e);
                        }
                        // Continue processing next fragments in this poll call.
                        Ok(ControlledPollAction::CONTINUE)

                    }


                }, 1 as i32);



                // if tx has space avail && poll does nothing then we should sleep
                // if cmd.vacant_units(&mut tx_lock.control_channel) >= 1 {
                //     //if there is space then we must have stopped due to no poll data therefore sleep
                //     //this is not async but does not consume CPU, it can do something else at this time
                //     //you may want to consider pinning or not pinning this to a core
                //     //you probably should NOT team this actor with others since they must wait for this (can we detect this?)
                //      precise_sleep(100_000); //wait 100 microseconds (100_000 nanoseconds)
                // }

            }
            warn!("We just exited the receiver");
        }

        Ok(())
    } else {
        Err("Failed to get state guard".into())
    }
}


