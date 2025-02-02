use std::error::Error;
use std::time::{Duration, Instant};
use steady_state::*;

pub const TEST_ITEMS: usize = 200_000_000;


pub const STREAM_ID: i32 = 11;
pub async fn run<const GIRTH:usize>(mut context: SteadyContext
                                    , rx: SteadyStreamRxBundle<StreamSessionMessage, GIRTH>) -> Result<(), Box<dyn Error>> {

    let mut cmd = into_monitor!(context, RxMetaDataHolder::new(rx.control_meta_data()), []);
    let mut rx = rx.lock().await;

    let data1 = Box::new([1, 2, 3, 4, 5, 6, 7, 8]);
    let data2 = Box::new([9, 10, 11, 12, 13, 14, 15, 16]);

    const LEN:usize = 100_000;

    let mut buffer: [StreamData<StreamSessionMessage>; LEN] = core::array::from_fn(|_| {
        StreamData::new(
            StreamSessionMessage::new(0, 0, Instant::now(), Instant::now()),
            Vec::new().into()
        )
    });

    let mut received_count = 0;
    while cmd.is_running(&mut || rx.is_closed_and_empty()) {


        let _clean = await_for_all!(cmd.wait_closed_or_avail_message_stream(&mut rx, LEN, 1));

        let taken = cmd.take_stream_slice::<LEN, StreamSessionMessage>(&mut rx[0], &mut buffer);
  //TODO: get connectivity working then add value testing ehre.


       //TODO: should work with both tominimize methods?
        // let avail = cmd.avail_units(&mut rx[0].item_channel);
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
        // let taken = avail;


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
