
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;

use std::error::Error;
use steady_state::steady_rx::{RxBundleTrait, SteadyRxBundleTrait};
use crate::actor::tick_consumer::TickCount;

#[cfg(not(test))]
pub async fn run<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyContext
        ,tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, tick_counts_rx).await
}

#[cfg(test)]
pub async fn run<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyContext
                                                    ,rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(),Box<dyn Error>> {
    let monitor = context.into_monitor(rx.meta_data(),[]);
    monitor.simulated_behavior(vec!(&TestEquals(rx[0].clone()))).await
             
}


const BATCH: usize = 1000;

async fn internal_behavior<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyContext, tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(), Box<dyn Error>> {
    let _cli_args = context.args::<Args>();

    let mut monitor = context.into_monitor( tick_counts_rx.meta_data(), []);

    let mut tick_counts_rx = tick_counts_rx.lock().await;
    let mut buffer = [TickCount::default(); BATCH];

    let mut my_max_count: u128 = 0;
    while monitor.is_running(&mut || tick_counts_rx.is_closed_and_empty()) {

        let _clean = await_for_all!(monitor.wait_avail_bundle(&mut tick_counts_rx, 1, TICK_COUNTS_RX_GIRTH)    );

        for i in 0..TICK_COUNTS_RX_GIRTH {
            let count = monitor.try_peek_slice(&mut tick_counts_rx[i], &mut buffer);
            if count > 0 {
                my_max_count = buffer[count - 1].count.max(my_max_count);
                for _n in 0..count {
                    let _ = monitor.try_take(&mut tick_counts_rx[i]).expect("internal error");
                }
            }
        }
        monitor.relay_stats_smartly();
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod actor_tests {
    use std::time::Duration;
    use async_std::test;
    use steady_state::*;
    use crate::actor::final_consumer::{internal_behavior, BATCH};
    use crate::actor::tick_consumer::TickCount;

    #[test]
    pub(crate) async fn test_simple_process() {
        //1. build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing().build(());
        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
                                            .with_capacity(BATCH)
                                            .build_as_bundle::<_, 3>();

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context| internal_behavior(context, ticks_rx_in.clone()) );

        graph.start();
        graph.request_stop();

        let test_data:Vec<TickCount> = (0..BATCH).map(|i| TickCount { count: i as u128 }).collect();

        ticks_tx_in[0].testing_send_all(test_data, true).await;
        ticks_tx_in[1].testing_close(Duration::from_millis(10)).await;
        ticks_tx_in[2].testing_close(Duration::from_millis(10)).await;
            
        ticks_tx_in.clone();
        graph.block_until_stopped(Duration::from_secs(240));

    }
}