use std::error::Error;
use std::time::Duration;
use log::info;
use steady_state::*;

pub const TEST_ITEMS: usize = 200_000_000;


pub const STREAM_ID: i32 = 11;

pub async fn run<const GIRTH: usize>(mut context: SteadyContext
                                     , tx: SteadyStreamTxBundle<StreamSimpleMessage, GIRTH>) -> Result<(), Box<dyn Error>>  {

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
                                       , vacant_items, vacant_bytes, 1));

        let mut remaining = TEST_ITEMS;
        let idx:usize = (STREAM_ID - tx[0].stream_id) as usize;
        while remaining > 0 && cmd.vacant_units(&mut tx[idx].item_channel) >= BATCH_SIZE {

            //TODO: make this interface also a trait.
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
