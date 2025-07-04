use std::error::Error;
use steady_state::*;

pub const TEST_ITEMS: usize = 20_000_000_000;

pub async fn run<const GIRTH:usize>(context: SteadyActorShadow
                                    , rx: SteadyStreamRxBundle<StreamIngress, GIRTH>) -> Result<(), Box<dyn Error>> {

    let mut actor = context.into_spotlight(rx.control_meta_data(), []);
    let mut rx = rx.lock().await;

    let data1 = Box::new([1, 2, 3, 4, 5, 6, 7, 8]);
    let data2 = Box::new([9, 10, 11, 12, 13, 14, 15, 16]);

    const LEN:usize = 100_000;


    let mut received_count = 0;
    while actor.is_running(&mut || rx.is_closed_and_empty()) {
        let _clean = await_for_all!(actor.wait_avail_bundle(&mut rx, LEN, 1));

         let avail = actor.avail_units(&mut rx[0]).0;

        for _z in 0..(avail>>1) {
            if let Some((i,d)) = actor.try_take(&mut rx[0]) {
                debug_assert_eq!(8, i.length);
                debug_assert_eq!(&*data1, &*d);
            }
            if let Some((i,d)) = actor.try_take(&mut rx[0]) {
                debug_assert_eq!(8, i.length);
                debug_assert_eq!(&*data2, &*d);
            }
        }
        let taken = avail;
        received_count += taken;

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
