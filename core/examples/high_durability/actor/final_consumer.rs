
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;

use std::error::Error;
use crate::actor::tick_consumer::TickCount;

#[cfg(not(test))]
pub async fn run<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyContext
        ,tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, tick_counts_rx).await
}

#[cfg(test)]
pub async fn run<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyContext
                                                    ,tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, tick_counts_rx).await
}


const BATCH: usize = 1000;

async fn internal_behavior<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyContext, tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(), Box<dyn Error>> {
    let _cli_args = context.args::<Args>();

    let mut monitor = into_monitor!(context, tick_counts_rx, []);

    let mut tick_counts_rx = tick_counts_rx.lock().await;
    let mut buffer = [TickCount::default(); BATCH];

    let mut my_max_count: u128 = 0;
    while monitor.is_running(&mut || tick_counts_rx.is_closed_and_empty()) {

        let _clean = wait_for_all!(monitor.wait_avail_units_bundle(&mut tick_counts_rx, 1, TICK_COUNTS_RX_GIRTH)    ).await;

        for i in 0..TICK_COUNTS_RX_GIRTH {
            let count = monitor.try_peek_slice(&mut tick_counts_rx[i], &mut buffer);
            if count > 0 {
                my_max_count = buffer[count - 1].count.max(my_max_count);
                for n in 0..count {
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
    use crate::actor::final_consumer::{BATCH, internal_behavior};
    use crate::actor::tick_consumer::TickCount;

    #[test]
    pub(crate) async fn test_simple_process() {
        //1. build test graph, the input and output channels and our actor
        let mut graph = Graph::new_test(());
        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
                                            .with_capacity(BATCH)
                                            .build_as_bundle::<_, 3>();

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context| internal_behavior(context, ticks_rx_in.clone()) );

        //2. add test data to the input channels
        let test_data:Vec<TickCount> = (0..BATCH).map(|i| TickCount { count: i as u128 }).collect();
        ticks_tx_in.clone();
        ticks_tx_in.testing_send(test_data, 0, true).await;
        ticks_tx_in.testing_mark_closed(1).await;
        ticks_tx_in.testing_mark_closed(2).await;

        //3. run graph until the actor detects the input is closed
        graph.start_as_data_driven(Duration::from_secs(240));

    }
}