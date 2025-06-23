
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;

use std::error::Error;

#[derive(Default, Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tick {
  pub value: u128
}

pub async fn run<const TICKS_TX_GIRTH:usize>(context: SteadyActorShadow
                                                            ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {
    let actor = context.into_spotlight([], ticks_tx.meta_data());
    if cfg!(not(test)) {
        internal_behavior(actor, ticks_tx).await
    } else {
        actor.simulated_behavior(vec!(&ticks_tx[0].clone())).await
    }
}

const BUFFER_SIZE:usize = 2000;

async fn internal_behavior<const TICKS_TX_GIRTH:usize,C: SteadyActor>(mut actor: C
                                                                      , ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let _cli_args = actor.args::<Args>();

    let mut ticks_tx = ticks_tx.lock().await;
    let batch = ticks_tx.capacity()/4;
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

#[cfg(test)]
pub(crate) mod actor_tests {
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::*;
    use super::*;

    #[test]
    fn test_tick_generator() {
        let mut graph = GraphBuilder::for_testing().build(());

        let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
            .with_capacity(BUFFER_SIZE)
            .build_channel_bundle::<_, 3>();

        graph.actor_builder()
            .with_name("UnitTest")
            .build( move |context| internal_behavior(context, ticks_tx_out.clone()), SoloAct );

        graph.start(); //startup the graph
        sleep(Duration::from_millis(40)); //if too long telemetry will back up
        
        graph.request_shutdown(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(15));

        assert_steady_rx_eq_count!(&ticks_rx_out[0],BUFFER_SIZE);
        assert_steady_rx_eq_count!(&ticks_rx_out[1],BUFFER_SIZE);
        assert_steady_rx_eq_count!(&ticks_rx_out[2],BUFFER_SIZE);
    }
}