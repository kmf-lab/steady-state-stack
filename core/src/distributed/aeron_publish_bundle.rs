use std::error::Error;
use std::sync::Arc;
use futures_timer::Delay;
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::{AlignedBuffer, AtomicBuffer};
use aeron::exclusive_publication::ExclusivePublication;
use aeron::utils::types::Index;
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::distributed_stream::{SteadyStreamRxBundle, SteadyStreamRxBundleTrait, StreamRxBundleTrait, StreamEgress};
use crate::{SteadyActor, SteadyState};
use crate::*;
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::simulate_edge::IntoSimRunner;
//  https://github.com/real-logic/aeron/wiki/Best-Practices-Guide


#[derive(Default)]
pub struct AeronPublishSteadyState {
    pub(crate) pub_reg_id: Vec<Option<i64>>,
    pub(crate) _items_taken: usize,
}

pub async fn run<const GIRTH:usize,>(context: SteadyActorShadow
                                     , rx: SteadyStreamRxBundle<StreamEgress,GIRTH>
                                     , aeron_connect: Channel
                                     , stream_id: i32
                                     , state: SteadyState<AeronPublishSteadyState>
                                     ) -> Result<(), Box<dyn Error>> {
    let mut actor = context.into_spotlight(rx.control_meta_data(), []);

    if actor.use_internal_behavior {
        while actor.aeron_media_driver().is_none() {
            warn!("unable to find Aeron media driver, will try again in 15 sec");
            let mut rx = rx.lock().await;
            if actor.is_running( &mut || rx.is_closed_and_empty() ) {
                let _ = actor.wait_periodic(Duration::from_secs(15)).await;
            } else {
                return Ok(());
            }
        }
        let aeron_media_driver = actor.aeron_media_driver().expect("media driver");
        return internal_behavior(actor, rx, aeron_connect, stream_id, aeron_media_driver, state).await;
    }
    let te:Vec<_> = rx.iter()
        .map(|f| f.clone() ).collect();
    let sims:Vec<_> = te.iter()
        .map(|f| f as &dyn IntoSimRunner<_>).collect();
    actor.simulated_behavior(sims).await
}



async fn internal_behavior<const GIRTH:usize,C: SteadyActor>(mut actor: C
                                                             , rx: SteadyStreamRxBundle<StreamEgress,GIRTH>
                                                             , aeron_channel: Channel
                                                             , stream_id: i32
                                                             , aeron:Arc<futures_util::lock::Mutex<Aeron>>
                                                             , state: SteadyState<AeronPublishSteadyState>) -> Result<(), Box<dyn Error>> {

    let mut rx = rx.lock().await;
    let mut state = state.lock(|| AeronPublishSteadyState::default()).await;
        let mut pubs: [ Result<ExclusivePublication, Box<dyn Error> >;GIRTH] = std::array::from_fn(|_| Err("Not Found".into())  );
        let mut last_position: [i64;GIRTH] = [0;GIRTH];

        //ensure right length
        while state.pub_reg_id.len()<GIRTH {
            state.pub_reg_id.push(None);
        }
        {
           let mut aeron = aeron.lock().await;  //other actors need this so do our work quick

           //trace!("holding add_exclusive_publication lock");
            for f in 0..GIRTH {
                if state.pub_reg_id[f].is_none() { //only add if we have not already done this
                    trace!("adding new pub {} {:?}",f as i32 +stream_id,aeron_channel.cstring() );
                    match aeron.add_exclusive_publication(aeron_channel.cstring(), f as i32 + stream_id) {
                        Ok(reg_id) => state.pub_reg_id[f] = Some(reg_id),
                        Err(e) => {
                            warn!("Unable to add publication: {:?}",e);
                        }
                    };
                }
            };
            //trace!("released add_exclusive_publication lock");
        }
        Delay::new(Duration::from_millis(2)).await; //back off so our request can get ready

        // now lookup when the publications are ready
        for f in 0..GIRTH {
            if let Some(id) = state.pub_reg_id[f] {
                let mut found = false;
                while actor.is_running(&mut || rx.is_closed_and_empty()) && !found {
                    let ex_pub = {
                        let mut aeron = aeron.lock().await;
                        aeron.find_exclusive_publication(id)
                    };
                    match ex_pub {
                        Err(e) => {
                            if e.to_string().contains("Awaiting")
                                || e.to_string().contains("not ready") {
                                //important that we do not poll fast while driver is setting up
                                Delay::new(Duration::from_millis(4)).await;
                                if actor.is_liveliness_stop_requested() {
                                    //trace!("stop detected before finding publication");
                                    //we are done, shutdown happened before we could start up.
                                    pubs[f] = Err("Shutdown requested while waiting".into());
                                    found = true;
                                }
                            } else {
                                warn!("Error finding publication: {:?}", e);
                                pubs[f] = Err(e.into());
                                found = true;
                            }
                        },
                        Ok(publication) => {  // Take ownership of the Arc and unwrap it
                            match Arc::try_unwrap(publication) {
                                Ok(mutex) => {   // Take ownership of the inner Mutex
                                    match mutex.into_inner() {

                                        Ok(publication) => {

                                            pubs[f] = Ok(publication);
                                            found = true;
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

        trace!("running publish '{:?}' all publications in place",actor.identity());

        let wait_for = 1;//(512*1024).min(rx.capacity());
        let in_channels = 1;

    //TODO: need to do this for the single publish
    //TODO: can we poll less if we see they are not flushing?
    
    //TODO: not good enought as we still loose th elast message, we reuqire waiting for subs to let go?

    let mut a_counters = [0;GIRTH];
    let mut b_counters = [0;GIRTH];
    let mut c_counters = [0;GIRTH];
    let mut d_counters = [0;GIRTH];



    let mut all_streams_flushed = false;
        while actor.is_running(&mut || rx.is_closed_and_empty() && all_streams_flushed) {

            let _clean = await_for_any!(
                           actor.wait_periodic(Duration::from_millis(16))
                          ,actor.wait_avail_bundle(&mut rx, wait_for, in_channels)
                         );

            let mut flushed_count = 0;
            for index in 0..GIRTH {
                match &mut pubs[index] {
                    Ok(p) => {
                        //trace!("AA {} stream:{}",i, p.stream_id());

                        loop {
                            let mut count_done = 0;
                            let mut count_bytes = 0;
                            let vacant_aeron_bytes = p.available_window().unwrap_or(0);
                            if vacant_aeron_bytes > 0 {
                                rx[index].consume_messages(&mut actor, vacant_aeron_bytes as usize, |mut slice1: &mut [u8], slice2: &mut [u8]| {
                                    let msg_len = slice1.len() + slice2.len();
                                    assert!(msg_len > 0);
                                    let response = if slice2.is_empty() {
                                        a_counters[index] += 1;

                                        p.offer_part(AtomicBuffer::wrap_slice(&mut slice1), 0, msg_len as Index)
                                    } else {  // TODO: p.try_claim() is probably a beter  way to move our datarather than AtomicBuffer usage..
                                        b_counters[index] += 1;

                                        let a_len = msg_len.min(slice1.len());
                                        let remaining_read = msg_len - a_len;
                                        let aligned_buffer = AlignedBuffer::with_capacity(msg_len as Index);
                                        let buf = AtomicBuffer::from_aligned(&aligned_buffer);
                                        buf.put_bytes(0, slice1);
                                        let b_len = remaining_read.min(slice2.len());
                                        let _extended_read = remaining_read - b_len;

                                        buf.put_bytes(a_len as Index, slice2);
                                        p.offer_part(buf, 0, msg_len as Index)
                                    };
                                    match response {
                                        Ok(value) => {
                                            c_counters[index] += 1;
                                            if value>=0 {
                                                last_position[index]= value;
                                                count_done += 1;
                                                count_bytes += msg_len;
                                                true
                                            } else {
                                                false
                                            }
                                        }
                                        Err(_aeron_error) => {
                                            d_counters[index] += 1;
                                            false
                                        }
                                    }
                                });
                            }
                            if 0==count_done {
                                break;
                            }
                        }

                        if let Ok(position) = p.position() {
                            if rx[index].is_closed_and_empty() {
                                    if position >= last_position[index] {
                                        error!("\nA totals {:?} \nB totals {:?} \nC totals {:?}\nD totals {:?}",a_counters,b_counters,c_counters,d_counters);

                                        if !p.is_connected() { // Wait until subscribers disconnect
                                            flushed_count += 1;
                                        }
                                    }
                            }

                        } else {
                            error!("error getting position");
                            //is closed
                            flushed_count +=1; //not sure...
                        }
                    }
                    Err(e) => {
                        warn!("{}",e);
                        //flushed_count +=1; //not sure...
                        yield_now().await;
                    }
                }
            }
            all_streams_flushed = GIRTH == flushed_count;
         }

    // After loop, ensure all messages are delivered
    for index in 0..GIRTH {
        if let Ok(p) = &mut pubs[index] {
            while p.position().unwrap_or(0) < last_position[index] || p.is_connected() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            p.close(); // Explicitly close the publication
        }
    } // TODO: if this does not work then we need a zombie check


    Ok(())
}



#[cfg(test)]
pub(crate) mod aeron_publish_bundle_tests {
    use super::*;
    use crate::distributed::aeron_channel_structs::{Endpoint, MediaType};
    use crate::distributed::aeron_channel_builder::{AeronConfig, AqueTech};
    use crate::distributed::distributed_builder::AqueductBuilder;
    use crate::distributed::distributed_stream::{SteadyStreamTxBundle, SteadyStreamTxBundleTrait, StreamIngress, StreamTxBundleTrait};
    use crate::distributed::distributed_stream::{LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundleClone, StreamEgress};

    //NOTE: bump this up for longer running load tests
    //       20_000_000_000;
    pub const TEST_ITEMS: usize = 200_000_000;


    pub const STREAM_ID: i32 = 11;
    //TODO: review the locking and init of terms in shared context??
    // The max length of a term buffer is 1GB (ie 1024MB) Imposed by the media driver.
    pub const _TERM_MB: i32 = 64; //at 1MB we are targeting 12M messages per second
       //our goal is to clear 39M messages per second requiring 4MB
       // a single stream at 64 maps 400MB of live shared memory
    // https://github.com/real-logic/aeron/wiki/Best-Practices-Guide
    // Check SO_RCVBUF and SO_SNDBUF settings on the NICs and the OS
    
    // for loopback testing, we may need queue length to hold more units for 4MB of buffer data
    // ip link show lo | grep qlen
    // sudo ip link set lo txqueuelen 10000

    // sudo ss -tulnpe | grep -E "$(docker inspect -f '{{.State.Pid}}' aeronmd)"
    // sudo ss -m -p | grep -E "$(docker inspect -f '{{.State.Pid}}' aeronmd)"

    pub async fn mock_sender_run<const GIRTH: usize>(context: SteadyActorShadow
                                                     , tx: SteadyStreamTxBundle<StreamEgress, GIRTH>) -> Result<(), Box<dyn Error>> {

        let mut actor = context.into_spotlight([], tx.control_meta_data());
        let mut tx = tx.lock().await;

        let data1 = [1, 2, 3, 4, 5, 6, 7, 8];
        let data2 = [9, 10, 11, 12, 13, 14, 15, 16];

        const BATCH_SIZE:usize = 5000;
        let items: [StreamEgress; BATCH_SIZE] = [StreamEgress::new(8);BATCH_SIZE];
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
        while actor.is_running(&mut || tx.mark_closed()) {

            //waiting for at least 1 channel in the stream has room for 2 made of 6 bytes
            let vacant_items = 200000;
            let data_size = 8;
            let vacant_bytes = vacant_items * data_size;

            let _clean = await_for_all!(actor.wait_vacant_bundle(&mut tx
                                       , (vacant_items, vacant_bytes), 1));

            let mut remaining = TEST_ITEMS;
            let idx:usize = (0 - STREAM_ID) as usize;
            while remaining > 0 && actor.vacant_units(&mut tx[idx].item_channel) >= BATCH_SIZE {

                //actor.send_stream_slice_until_full(&mut tx, STREAM_ID, &items, &all_bytes );
                actor.send_slice_until_full(&mut tx[idx].payload_channel, &all_bytes);
                actor.send_slice_until_full(&mut tx[idx].item_channel, &items);

                // this old solution worked but consumed more core
                // for _i in 0..(actual_vacant >> 1) { //old code, these functions are important
                //     let _result = actor.try_stream_send(&mut tx, STREAM_ID, &data1);
                //     let _result = actor.try_stream_send(&mut tx, STREAM_ID, &data2);
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

    pub async fn mock_receiver_run<const GIRTH:usize>(context: SteadyActorShadow
                                                      , rx: SteadyStreamRxBundle<StreamIngress, GIRTH>) -> Result<(), Box<dyn Error>> {

        let mut actor = context.into_spotlight(rx.control_meta_data(), []);
        let mut rx = rx.lock().await;

        let _data1 = Box::new([1, 2, 3, 4, 5, 6, 7, 8]);
        let _data2 = Box::new([9, 10, 11, 12, 13, 14, 15, 16]);

        const LEN:usize = 100_000;

        // let mut buffer: [StreamData<StreamSessionMessage>; LEN] = core::array::from_fn(|_| {
        //     StreamData::new(
        //         StreamSessionMessage::new(0, 0, Instant::now(), Instant::now()),
        //         Vec::new().into()
        //     )
        // });

        let mut received_count = 0;
        while actor.is_running(&mut || rx.is_closed_and_empty()) {

            let _clean = await_for_all!(actor.wait_avail_bundle(&mut rx, LEN, 1));

            //we waited above for 2 messages so we know there are 2 to consume
            //reading from a single channel with a single stream id

            //let taken = actor.take_stream_slice::<LEN, StreamSessionMessage>(&mut rx[0], &mut buffer);

            let bytes = actor.avail_units(&mut rx[0].payload_channel);
            actor.advance_read_index(&mut rx[0].payload_channel, bytes);
            let taken = actor.avail_units(&mut rx[0].item_channel);
            actor.advance_read_index(&mut rx[0].item_channel, taken);


            //  let avail = actor.avail_units(&mut rx[0].item_channel);

           // TODO: need a way to test this..


            // for i in 0..(avail>>1) {
            //     if let Some(d) = actor.try_take_stream(&mut rx[0]) {
            //         //warn!("test data {:?}",d.payload);
            //         debug_assert_eq!(&*data1, &*d.payload);
            //     }
            //     if let Some(d) = actor.try_take_stream(&mut rx[0]) {
            //         //warn!("test data {:?}",d.payload);
            //         debug_assert_eq!(&*data2, &*d.payload);
            //     }
            // }


            received_count += taken;
            //actor.relay_stats_smartly(); //should not be needed.

            //here we request shutdown but we only leave after our upstream actors are done
            if received_count >= (TEST_ITEMS-taken) {
                error!("stop requested");
                actor.request_shutdown().await;
                return Ok(());
            }
        }

        error!("receiver is done");
        Ok(())
    }

    #[async_std::test]
    async fn test_bytes_process() -> Result<(), Box<dyn Error>> {
        if true {
             return Ok(()); //do not run this test
        }
        if std::env::var("GITHUB_ACTIONS").is_ok() {
            return Ok(());
        }

        let mut graph = GraphBuilder::for_testing()
            .with_telemetry_metric_features(true)
            .build(());

        let aeron_md = graph.aeron_media_driver();
        if aeron_md.is_none() {
            info!("aeron test skipped, no media driver present");
            return Ok(());
        }

        let channel_builder = graph.channel_builder();

        let (to_aeron_tx,to_aeron_rx) = channel_builder
            .with_avg_rate()
            .with_avg_filled()
            .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
            .with_capacity(4*1024*1024)
            .build_stream_bundle::<StreamEgress,1>(8);

        let (from_aeron_tx,from_aeron_rx) = channel_builder
            .with_avg_rate()
            .with_avg_filled()
            .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
            .with_capacity(4*1024*1024)
            .build_stream_bundle::<StreamIngress,1>(8);

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


        graph.actor_builder().with_name("MockSender")
            .with_thread_info()
            .with_mcpu_percentile(Percentile::p96())
            .with_mcpu_percentile(Percentile::p25())

            //  .with_explicit_core(6)
            .build(move |context| mock_sender_run(context, to_aeron_tx.clone())
                   , ScheduleAs::SoloAct);

        let stream_id = 12;

        to_aeron_rx.build_aqueduct(AqueTech::Aeron(aeron_config.clone(), stream_id)
                                   , &graph.actor_builder().with_name("SenderTest").never_simulate(true)
                                   , ScheduleAs::SoloAct);

        //set this up first so sender has a place to send to
        graph.actor_builder().with_name("MockReceiver")
            .with_thread_info()
            .with_mcpu_percentile(Percentile::p96())
            .with_mcpu_percentile(Percentile::p25())

            // .with_explicit_core(9)
            .build(move |context| mock_receiver_run(context, from_aeron_rx.clone())
                   , ScheduleAs::SoloAct);

        from_aeron_tx.build_aqueduct(AqueTech::Aeron(aeron_config.clone(), stream_id)
                                     , &graph.actor_builder().with_name("ReceiverTest").never_simulate(true)
                                     , ScheduleAs::SoloAct);

        graph.start(); //startup the graph
        graph.block_until_stopped(Duration::from_secs(21))
    }

}
