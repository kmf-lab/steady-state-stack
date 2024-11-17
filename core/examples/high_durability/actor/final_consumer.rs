
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;

use std::error::Error;
use steady_state::commander::SteadyCommander;
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
    let mut monitor = into_monitor!(context,rx,[]);
    let mut rx = rx.lock().await;

    if let Some(simulator) = monitor.sidechannel_responder() {
        while monitor.is_running(&mut || rx.is_closed_and_empty()) {

            let _clean = await_for_all!(monitor.wait_shutdown_or_avail_units(&mut rx[0],1));
            simulator.respond_with(|expected| {
                match monitor.try_take(&mut rx[0]) {
                    Some(measured) => {
                        let expected: &TickCount = expected.downcast_ref::<TickCount>().expect("error casting");

                        if expected.cmp(&measured).is_eq() {
                            Box::new("ok".to_string())
                        } else {
                            let failure = format!("no match {:?} {:?}"
                                                  , expected
                                                  , measured).to_string();
                            error!("failure: {}", failure);
                            Box::new(failure)
                        }

                    },
                    None => Box::new("no data".to_string()),
                }

            }).await;
        }

    }

    Ok(())
}


const BATCH: usize = 1000;

async fn internal_behavior<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyContext, tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(), Box<dyn Error>> {
    let _cli_args = context.args::<Args>();

    let mut monitor = into_monitor!(context, tick_counts_rx, []);

    let mut tick_counts_rx = tick_counts_rx.lock().await;
    let mut buffer = [TickCount::default(); BATCH];

    let mut my_max_count: u128 = 0;
    while monitor.is_running(&mut || tick_counts_rx.is_closed_and_empty()) {

        let _clean = await_for_all!(monitor.wait_shutdown_or_avail_units_bundle(&mut tick_counts_rx, 1, TICK_COUNTS_RX_GIRTH)    );

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
    use crate::actor::final_consumer::{BATCH, internal_behavior};
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

        ticks_tx_in[0].testing_send_in_two_batches(test_data, Duration::from_millis(10), true).await;
        ticks_tx_in[1].testing_close(Duration::from_millis(10)).await;
        ticks_tx_in[2].testing_close(Duration::from_millis(10)).await;
            
        ticks_tx_in.clone();
        graph.block_until_stopped(Duration::from_secs(240));

    }
}