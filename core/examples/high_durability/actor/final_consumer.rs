
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;

use std::error::Error;
use crate::actor::tick_consumer::TickCount;




pub async fn run<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyContext
        ,tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let _cli_args = context.args::<Args>();

    let mut monitor =  into_monitor!(context, tick_counts_rx, []);

    let mut tick_counts_rx = tick_counts_rx.lock().await;
    let mut buffer = [TickCount::default(); 1000];

    let mut my_max_count: u32 = 0; //keep outside..

    while monitor.is_running(&mut || tick_counts_rx.is_closed_and_empty()) { //TODO: fix code generator!!

         let _clean = wait_for_all!(monitor.wait_avail_units_bundle(&mut tick_counts_rx, 1, TICK_COUNTS_RX_GIRTH)    ).await;

         for i in 0..TICK_COUNTS_RX_GIRTH {

             let count = monitor.try_peek_slice(&mut tick_counts_rx[i], &mut buffer);

             if count>0 {
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

/*
#[cfg(test)]
mod tests {
    use async_std::test;
    use steady_state::*;


    #[test]
    async fn test_process() {
        util::logger::initialize();
        let mut graph = Graph::new(());

        //build your channels as needed for testing
        let (tx, rx) = graph.channel_builder().with_capacity(8).build();
         let (tick_counts_tx_extern, tick_counts_rx) = graph.channel_builder().with_capacity(8).build();
         let mock_context = graph.new_test_monitor("mock");
         let mut mock_monitor = into_monitor!(mock_context, [], []);
         let mut tick_counts_tx_extern = tick_counts_tx_extern.lock().await;
         let mut tick_counts_rx = tick_counts_rx.lock().await;

    }
}
*/