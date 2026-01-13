#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;

use std::error::Error;
use steady_state::steady_rx::{RxBundleTrait, SteadyRxBundleTrait};
use crate::actor::tick_consumer::TickCount;

pub async fn run<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyActorShadow
        ,tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(),Box<dyn Error>> {
    let actor= context.into_spotlight(tick_counts_rx.meta_data(), []);
    if actor.use_internal_behavior {
        internal_behavior(actor, tick_counts_rx).await
    } else {
        actor.simulated_behavior(sim_runners!(tick_counts_rx)).await
    }
}

const BATCH: usize = 1000;

async fn internal_behavior<const TICK_COUNTS_RX_GIRTH:usize,C: SteadyActor>(mut actor: C, tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(), Box<dyn Error>> {
    let _cli_args = actor.args::<Args>();

    let mut tick_counts_rx = tick_counts_rx.lock().await;
    let mut buffer = [TickCount::default(); BATCH];

    let mut my_max_count: u128 = 0;
    while actor.is_running(&mut || tick_counts_rx.is_closed_and_empty()) {

        let _clean = await_for_all!(actor.wait_avail_bundle(&mut tick_counts_rx, 1, TICK_COUNTS_RX_GIRTH)    );

        for i in 0..TICK_COUNTS_RX_GIRTH {
            let slice = actor.peek_slice(&mut tick_counts_rx[i]);
            let count = slice.copy_into_slice(&mut buffer).item_count();
            if count > 0 {
                my_max_count = buffer[count - 1].count.max(my_max_count);
                for _n in 0..count {
                    let _ = actor.try_take(&mut tick_counts_rx[i]).expect("internal error");
                }
            }
        }
        actor.relay_stats_smartly();
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod actor_tests {
    use std::time::Duration;
    use steady_state::*;
    use crate::actor::final_consumer::{internal_behavior, BATCH};
    use crate::actor::tick_consumer::TickCount;

    #[test]
    fn test_final_consumer() -> Result<(), Box<dyn Error>> {
        //1. build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing().build(());
        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
                                            .with_capacity(BATCH)
                                            .build_channel_bundle::<_, 3>();

        graph.actor_builder()
            .with_name("UnitTest")
            .build( move |context| internal_behavior(context, ticks_rx_in.clone()) , SoloAct);

        graph.start();
        graph.request_shutdown();

        let test_data:Vec<TickCount> = (0..BATCH).map(|i| TickCount { count: i as u128 }).collect();

        ticks_tx_in[0].testing_send_all(test_data, true);
        ticks_tx_in[1].testing_close();
        ticks_tx_in[2].testing_close();
            
        ticks_tx_in.clone();
        graph.block_until_stopped(Duration::from_secs(240))

    }
}
