
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;
use std::error::Error;
use steady_state::steady_tx::TxMetaDataProvider;

#[derive(Default,Clone,Copy,Debug,Eq,PartialEq)]
pub struct Tick {
  pub value: u128
}

pub async fn run<const TICKS_TX_GIRTH:usize,>(context: SteadyContext
                                                            ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {
    let cmd = context.into_monitor( [], ticks_tx.meta_data());
    if cfg!(not(test)) {
        internal_behavior(cmd, ticks_tx).await
    } else {
        cmd.simulated_behavior(vec!(&ticks_tx[0])).await
    }
}

#[allow(unused)]
const BUFFER_SIZE:usize = 1000;

//tag that it is ok that this is never called
#[allow(unused)]
async fn internal_behavior<const TICKS_TX_GIRTH:usize,C: SteadyCommander>(mut cmd: C
        ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let _cli_args = cmd.args::<Args>();

    let mut ticks_tx = ticks_tx.lock().await;
    let batch = ticks_tx.capacity()/8;
    let mut buffers:[Tick; BUFFER_SIZE] = [Tick { value: 0 }; BUFFER_SIZE];

    let mut count: u128 = 0;
    while cmd.is_running(&mut || ticks_tx.mark_closed()) {
         let _clean = await_for_all!(cmd.wait_vacant_bundle(&mut ticks_tx, batch, TICKS_TX_GIRTH)    );
         for i in 0..TICKS_TX_GIRTH {
             let c = ticks_tx[i].vacant_units().min(BUFFER_SIZE);
             for n in 0..c {
                 count = count + 1;
                 buffers[n] = Tick { value: count };
             }
             cmd.send_slice_until_full(&mut ticks_tx[i], &buffers[..c]);
         }
        cmd.relay_stats_smartly();
    }
    Ok(())
}

