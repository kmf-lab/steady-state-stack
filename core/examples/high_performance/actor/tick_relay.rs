
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;
use std::error::Error;
use crate::actor::tick_generator::Tick;

pub async fn run(context: SteadyContext
        ,ticks_rx: SteadyRx<Tick>
        ,ticks_tx: SteadyTx<Tick>) -> Result<(),Box<dyn Error>> {

    let _cli_args = context.args::<Args>();

    let mut monitor =  into_monitor!(context, [ticks_rx],[ticks_tx]);

    let mut ticks_rx = ticks_rx.lock().await;
    let mut ticks_tx = ticks_tx.lock().await;

    const BATCH:usize = 2000;
    let mut buffer = [Tick::default(); 2000];

    while monitor.is_running(&mut || ticks_rx.is_closed_and_empty() && ticks_tx.mark_closed()) {

         let _clean = wait_for_all!(
                                    monitor.wait_avail_units(&mut ticks_rx,BATCH),
                                    monitor.wait_vacant_units(&mut ticks_tx,BATCH)
                                   ).await;

        let count = monitor.take_slice(&mut ticks_rx, &mut buffer);
        //do something
        monitor.send_slice_until_full(&mut ticks_tx, &buffer[0..count]);

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
         let (ticks_tx_extern, ticks_rx) = graph.channel_builder().with_capacity(8).build();
         
         let (tick_counts_tx,tick_counts_rx_extern) = graph.channel_builder().with_capacity(8).build();
         let mock_context = graph.new_test_monitor("mock");
         let mut mock_monitor = into_monitor!(mock_context, [], []);
         let mut ticks_tx_extern = ticks_tx_extern.lock().await;
         let mut ticks_rx = ticks_rx.lock().await;
         
         let mut tick_counts_tx = tick_counts_tx.lock().await;
         let mut tick_counts_rx_extern = tick_counts_rx_extern.lock().await;



    }
}

 */