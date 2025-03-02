use std::error::Error;
use steady_state::*;

pub const TEST_ITEMS: usize = 20_000_000_000;
pub const STREAM_ID: i32 = 1234;

pub async fn run<const GIRTH: usize>(context: SteadyContext
                                     , tx: SteadyStreamTxBundle<StreamSimpleMessage, GIRTH>) -> Result<(), Box<dyn Error>>  {

    let mut cmd = context.into_monitor([], tx.control_meta_data());
    let mut tx = tx.lock().await;

    warn!("called run");

    let data1 = [1, 2, 3, 4, 5, 6, 7, 8];
    let data2 = [9, 10, 11, 12, 13, 14, 15, 16];

    const BATCH_SIZE:usize = 5000;

    let mut sent_count = 0;
    while cmd.is_running(&mut || tx.mark_closed()) {

        //waiting for at least 1 channel in the stream has room for 2 made of 6 bytes
        let vacant_items = 200000;
        let data_size = 8;
        let vacant_bytes = vacant_items * data_size;
// TODO: wrwrite to take (i,p) as a group.
        let _clean = await_for_all!(cmd.wait_vacant_bundle(&mut tx
                                       , (vacant_items, vacant_bytes), 1));


        let item = StreamSimpleMessage::new(8);

        let mut remaining = TEST_ITEMS;
         let idx:usize = (0 - STREAM_ID) as usize;
         // trace!("index of {} out of {}",idx, tx.len());
         while remaining > 0 && cmd.vacant_units(&mut tx[idx]) >= BATCH_SIZE {

             let actual_vacant = cmd.vacant_units(&mut tx[idx]);

             // TODO: auto convert to item..
             for _i in 0..(actual_vacant >> 1) { //old code, these functions are important
                                                              // item!(&data)
                 let _result = cmd.try_send(&mut tx[idx], StreamSimpleMessage::wrap(&data1) );
                 let _result = cmd.try_send(&mut tx[idx], (item,&data2));

             }
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
