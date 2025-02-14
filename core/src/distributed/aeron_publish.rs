use std::error::Error;
use std::sync::Arc;
use futures_timer::Delay;
use ringbuf::consumer::Consumer;
use ringbuf::traits::Observer;
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::{AlignedBuffer, AtomicBuffer};
use aeron::exclusive_publication::ExclusivePublication;
use aeron::utils::types::Index;
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::distributed_stream::{SteadyStreamRx, SteadyStreamRxBundle, SteadyStreamRxBundleTrait, StreamRxBundleTrait, StreamRxDef, StreamSimpleMessage};
use crate::{into_monitor, SteadyCommander, SteadyState};
use crate::*;
use crate::commander_context::SteadyContext;
use crate::monitor::RxMetaDataHolder;
//  https://github.com/real-logic/aeron/wiki/Best-Practices-Guide

const ITEMS_BUFFER_SIZE:usize = 1000;

#[derive(Default)]
pub(crate) struct AeronPublishSteadyState {
    pub(crate) pub_reg_id: Option<i64>,
    pub(crate) items_taken: usize,
}

pub async fn run(context: SteadyContext
             , rx: SteadyStreamRx<StreamSimpleMessage>
             , aeron_connect: Channel
             , aeron:Arc<futures_util::lock::Mutex<Aeron>>
             , state: SteadyState<AeronPublishSteadyState>) -> Result<(), Box<dyn Error>> {

    internal_behavior(into_monitor!(context, [rx.meta_data().control], [])
                      , rx, aeron_connect, aeron, state).await
}

async fn internal_behavior<C: SteadyCommander>(mut cmd: C
                                                                 , rx: SteadyStreamRx<StreamSimpleMessage>
                                                                 , aeron_channel: Channel
                                                                 , aeron:Arc<futures_util::lock::Mutex<Aeron>>
                                                                 , state: SteadyState<AeronPublishSteadyState>) -> Result<(), Box<dyn Error>> {

    let mut rx = rx.lock().await;


    let mut state_guard = steady_state(&state, || AeronPublishSteadyState::default()).await;
    if let Some(state) = state_guard.as_mut() {

        {
            let mut aeron = aeron.lock().await;  //other actors need this so do our work quick
           //trace!("holding add_exclusive_publication lock");
            if state.pub_reg_id.is_none() { //only add if we have not already done this
                warn!("adding new pub {} {:?}",rx.stream_id,aeron_channel.cstring() );
                match aeron.add_exclusive_publication(aeron_channel.cstring(), rx.stream_id) {
                    Ok(reg_id) => state.pub_reg_id = Some(reg_id),
                    Err(e) => {
                        warn!("Unable to add publication: {:?}",e);
                    }
                };
            }
        }
        Delay::new(Duration::from_millis(2)).await; //back off so our request can get ready

        // now lookup when the publications are ready
        let mut my_pub = Err("");
                if let Some(id) = state.pub_reg_id {
                    my_pub = loop {
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
                                    Delay::new(Duration::from_millis(4)).await;
                                    if cmd.is_liveliness_stop_requested() {
                                        //trace!("stop detected before finding publication");
                                        //we are done, shutdown happened before we could start up.
                                        break Err("Shutdown requested while waiting".into());
                                    }
                                } else {
                                    warn!("Error finding publication: {:?}", e);
                                    break Err("Unable to find requested publication".into());
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
                                                loop {
                                                    // warn!("pub {} max_message_length {} max_message_length {} available_window {:?}"
                                                    //       , publication.session_id(),  publication.max_message_length(), publication.max_message_length(), publication.available_window() );
                                                    // 
                                                    if let Ok(w) = publication.available_window() {
                                                        if w>1024 {
                                                            break; //we have a window!!
                                                        }
                                                        Delay::new(Duration::from_millis(40));
                                                    } else {
                                                        Delay::new(Duration::from_millis(200));
                                                    }
                                                }
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

        warn!("running publish '{:?}' all publications in place",cmd.identity());
        let capacity:usize = rx.capacity().into();
        let wait_for = (512*1024).min(capacity);

        let mut backoff = true;
        while cmd.is_running(&mut || rx.is_closed_and_empty()) {
    
            let clean = await_for_any!(cmd.wait_periodic(Duration::from_millis(10))
                                           ,cmd.wait_closed_or_avail_units(&mut rx, wait_for)
                           );


            let mut count_done = 0;
            let mut count_bytes = 0;

                backoff = false;

                    //buld a working batch solution first and then extract to functions later
                    //peek a block ahead, 

                       //provide every message and slice until false is returned at that point
                        //we release everything consumed up to this point and return or if no data
                        //upon return release
                    match &mut my_pub {
                        Ok(p) => {

                            let vacant_aeron_bytes = p.available_window().unwrap_or(0);
             //                   let mut _aeron = aeron.lock().await;  //other actors need this so do our work quick
                                rx.consume_messages(&mut cmd, vacant_aeron_bytes as usize, |mut slice1: &mut [u8], mut slice2: &mut [u8]| {
                                    let msg_len = slice1.len() + slice2.len();
                                    assert!(msg_len>0);
                                    let response = if slice2.len() == 0 {
                                        p.offer_part(AtomicBuffer::wrap_slice(&mut slice1), 0, msg_len as Index)
                                    } else {  // TODO: p.try_claim() is probably a beter  way to move our datarather than AtomicBuffer usage..
                                        let a_len = msg_len.min(slice1.len());
                                        let remaining_read = msg_len - a_len;
                                        let aligned_buffer = AlignedBuffer::with_capacity(msg_len as Index);
                                        let mut buf = AtomicBuffer::from_aligned(&aligned_buffer);
                                        buf.put_bytes(0, slice1);
                                        let b_len = remaining_read.min(slice2.len());
                                        let _extended_read = remaining_read - b_len;

                                        buf.put_bytes(a_len as Index, slice2);
                                        p.offer_part(buf, 0, msg_len as Index)
                                    };
                                    match response {
                                        Ok(_value) => {
                                            count_done += 1;
                                            count_bytes+=msg_len;
                                            true
                                        }
                                        Err(aeron_error) => {
                                            backoff = true;
                                            false
                                        }
                                    }
                                });
                        }
                        Err(e) => {
                            warn!("panic details {}",e);
                          //  panic!("{:?}", e); //we should have had the pup so try again
                        }
                    }
         }        
    }
    Ok(())
}

#[cfg(test)]
#[ignore] //too heavy weight for normal testing, a light version exists in aeron_subscribe
pub(crate) mod aeron_tests {
    use super::*;
    use crate::distributed::aeron_channel_structs::{Endpoint, MediaType};
    use crate::distributed::aeron_channel_builder::{AeronConfig, AqueTech};
    use crate::distributed::aeron_subscribe_bundle;
    use crate::distributed::distributed_stream::{SteadyStreamTxBundle, SteadyStreamTxBundleTrait, StreamSessionMessage, StreamTxBundleTrait};
    use crate::monitor::TxMetaDataHolder;
    use crate::distributed::distributed_stream::{LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundleClone, StreamSimpleMessage};

    //NOTE: bump this up for longer running load tests
    //       20_000_000_000;
    pub const TEST_ITEMS: usize = 200_000_000;

    pub const STREAM_ID: i32 = 11;
    //TODO: review the locking and init of terms in shared context??
    // The max length of a term buffer is 1GB (ie 1024MB) Imposed by the media driver.
    pub const TERM_MB: i32 = 64; //at 1MB we are targeting 12M messages per second
       //our goal is to clear 39M messages per second requiring 4MB
       // a single stream at 64 maps 400MB of live shared memory
    // https://github.com/real-logic/aeron/wiki/Best-Practices-Guide
    // Check SO_RCVBUF and SO_SNDBUF settings on the NICs and the OS
    
    // for loopback testing, we may need queue length to hold more units for 4MB of buffer data
    // ip link show lo | grep qlen
    // sudo ip link set lo txqueuelen 10000

    // sudo ss -tulnpe | grep -E "$(docker inspect -f '{{.State.Pid}}' aeronmd)"
    // sudo ss -m -p | grep -E "$(docker inspect -f '{{.State.Pid}}' aeronmd)"

    pub async fn mock_sender_run<const GIRTH: usize>(mut context: SteadyContext
                                                     , tx: SteadyStreamTxBundle<StreamSimpleMessage, GIRTH>) -> Result<(), Box<dyn Error>> {

        let mut cmd = into_monitor!(context, [], TxMetaDataHolder::new(tx.control_meta_data()));
        let mut tx = tx.lock().await;

        let data1 = [1, 2, 3, 4, 5, 6, 7, 8];
        let data2 = [9, 10, 11, 12, 13, 14, 15, 16];

        const BATCH_SIZE:usize = 5000;
        let mut items: [StreamSimpleMessage; BATCH_SIZE] = [StreamSimpleMessage::new(8);BATCH_SIZE];
        let mut data: [[u8;8]; BATCH_SIZE] = [data1; BATCH_SIZE];
        for i in 0..BATCH_SIZE {
            if i % 2 == 0 {
                data[i] = data1;
            } else {
                data[i] = data2;
            }
        }
        let all_bytes: Vec<u8> = data.iter().flatten().map(|f| *f).collect();

        let mut sent_count = 0;
        while cmd.is_running(&mut || tx.mark_closed()) {

            //waiting for at least 1 channel in the stream has room for 2 made of 6 bytes
            let vacant_items = 200000;
            let data_size = 8;
            let vacant_bytes = vacant_items * data_size;

            let _clean = await_for_all!(cmd.wait_shutdown_or_vacant_units_stream(&mut tx
                                       , (vacant_items, vacant_bytes), 1));

            let mut remaining = TEST_ITEMS;
            let idx:usize = (STREAM_ID - tx[0].stream_id) as usize;
            while remaining > 0 && cmd.vacant_units(&mut tx[idx].item_channel) >= BATCH_SIZE {

                //cmd.send_stream_slice_until_full(&mut tx, STREAM_ID, &items, &all_bytes );
                cmd.send_slice_until_full(&mut tx[idx].payload_channel, &all_bytes);
                cmd.send_slice_until_full(&mut tx[idx].item_channel, &items);

                // this old solution worked but consumed more core
                // for _i in 0..(actual_vacant >> 1) { //old code, these functions are important
                //     let _result = cmd.try_stream_send(&mut tx, STREAM_ID, &data1);
                //     let _result = cmd.try_stream_send(&mut tx, STREAM_ID, &data2);
                // }
                sent_count += BATCH_SIZE;
                remaining -= BATCH_SIZE
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

        let data1 = Box::new([1, 2, 3, 4, 5, 6, 7, 8]);
        let data2 = Box::new([9, 10, 11, 12, 13, 14, 15, 16]);

        const LEN:usize = 100_000;

        // let mut buffer: [StreamData<StreamSessionMessage>; LEN] = core::array::from_fn(|_| {
        //     StreamData::new(
        //         StreamSessionMessage::new(0, 0, Instant::now(), Instant::now()),
        //         Vec::new().into()
        //     )
        // });

        let mut received_count = 0;
        while cmd.is_running(&mut || rx.is_closed_and_empty()) {

            let _clean = await_for_all!(cmd.wait_closed_or_avail_message_stream(&mut rx, LEN, 1));

            //we waited above for 2 messages so we know there are 2 to consume
            //reading from a single channel with a single stream id

            //let taken = cmd.take_stream_slice::<LEN, StreamSessionMessage>(&mut rx[0], &mut buffer);

            let bytes = cmd.avail_units(&mut rx[0].payload_channel);
            cmd.advance_read_index(&mut rx[0].payload_channel, bytes);
            let taken = cmd.avail_units(&mut rx[0].item_channel);
            cmd.advance_read_index(&mut rx[0].item_channel, taken);


            //  let avail = cmd.avail_units(&mut rx[0].item_channel);

           // TODO: need a way to test this..


            // for i in 0..(avail>>1) {
            //     if let Some(d) = cmd.try_take_stream(&mut rx[0]) {
            //         //warn!("test data {:?}",d.payload);
            //         debug_assert_eq!(&*data1, &*d.payload);
            //     }
            //     if let Some(d) = cmd.try_take_stream(&mut rx[0]) {
            //         //warn!("test data {:?}",d.payload);
            //         debug_assert_eq!(&*data2, &*d.payload);
            //     }
            // }


            received_count += taken;
            //cmd.relay_stats_smartly(); //should not be needed.

            //here we request shutdown but we only leave after our upstream actors are done
            if received_count >= (TEST_ITEMS-taken) {
                error!("stop requested");
                cmd.request_graph_stop();
                return Ok(());
            }
        }

        error!("receiver is done");
        Ok(())
    }

    #[async_std::test]
   // #[ignore] //too heavy weight for normal testing, a light version exists in aeron_subscribe
    async fn test_bytes_process() {
        if true {
             return; //do not run this test
        }

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
            .with_capacity(4*1024*1024)
            .build_as_stream_bundle::<StreamSimpleMessage,1>(STREAM_ID
                                                             ,8);
        let (from_aeron_tx,from_aeron_rx) = channel_builder
            .with_avg_rate()
            .with_avg_filled()
            .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
            .with_capacity(4*1024*1024)
            .build_as_stream_bundle::<StreamSessionMessage,1>(STREAM_ID
                                                              , 8);

        //  https://github.com/real-logic/aeron/wiki/Best-Practices-Guide
        let aeron_config = AeronConfig::new()            
            //TODO: to hack Ipc we need point to point and no term length
            //we will use this for unit tests.
           .with_media_type(MediaType::Ipc) // 10MMps

        //    .with_media_type(MediaType::Udp)// 4MMps- std 4K page
          //  .with_term_length((1024 * 1024 * TERM_MB) as usize)

            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("Invalid IP address"),
                port: 40456,
            })
            .build();


        let dist =  AqueTech::Aeron(aeron_config);

        graph.actor_builder().with_name("MockSender")
            .with_thread_info()
            .with_mcpu_percentile(Percentile::p96())
            .with_mcpu_percentile(Percentile::p25())

            //  .with_explicit_core(6)
            .build(move |context| mock_sender_run(context, to_aeron_tx.clone())
                   , &mut Threading::Spawn);

        graph.build_stream_distributor_bundle(dist.clone()
                                       , "SenderTest"
                                       , to_aeron_rx
                                       , &mut Threading::Spawn);

        //set this up first so sender has a place to send to
        graph.actor_builder().with_name("MockReceiver")
            .with_thread_info()
            .with_mcpu_percentile(Percentile::p96())
            .with_mcpu_percentile(Percentile::p25())

            // .with_explicit_core(9)
            .build(move |context| mock_receiver_run(context, from_aeron_rx.clone())
                   , &mut Threading::Spawn);

        graph.build_stream_collector_bundle(dist.clone()
                                            , "ReceiverTest"
                                            , from_aeron_tx
                                            , &mut Threading::Spawn);

        graph.start(); //startup the graph
        graph.block_until_stopped(Duration::from_secs(21));
    }

}
