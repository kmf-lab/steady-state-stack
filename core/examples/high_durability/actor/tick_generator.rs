#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;
use std::error::Error;

#[derive(Default,Clone,Copy,Debug,Eq,PartialEq)]
pub struct Tick {
  pub value: u128
}

pub async fn run<const TICKS_TX_GIRTH:usize,>(context: SteadyActorShadow
                                                            ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {
    let actor = context.into_spotlight([], ticks_tx.meta_data());
    if actor.use_internal_behavior {
        internal_behavior(actor, ticks_tx).await
    } else {
        actor.simulated_behavior(sim_runners!(ticks_tx)).await
    }
}

#[allow(unused)]
const BUFFER_SIZE:usize = 1000;

//tag that it is ok that this is never called
#[allow(unused)]
async fn internal_behavior<const TICKS_TX_GIRTH:usize,C: SteadyActor>(mut actor: C
                                                                      , ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let _cli_args = actor.args::<Args>();

    let mut ticks_tx = ticks_tx.lock().await;
    let batch = ticks_tx.capacity()/8;
    let mut buffers:[Tick; BUFFER_SIZE] = [Tick { value: 0 }; BUFFER_SIZE];

    let mut count: u128 = 0;
    while actor.is_running(&mut || ticks_tx.mark_closed()) {
         let _clean = await_for_all!(actor.wait_vacant_bundle(&mut ticks_tx, batch, TICKS_TX_GIRTH)    );
         for i in 0..TICKS_TX_GIRTH {
             let c = ticks_tx[i].vacant_units().min(BUFFER_SIZE);
             for n in 0..c {
                 count += 1;
                 buffers[n] = Tick { value: count };
             }
             actor.send_slice(&mut ticks_tx[i], &buffers[..c]);
         }
        actor.relay_stats_smartly();
    }
    Ok(())
}
