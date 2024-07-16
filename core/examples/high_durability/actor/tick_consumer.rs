
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;



use std::error::Error;
use crate::actor::tick_generator::Tick;


#[derive(Default,Clone,Copy)]
pub struct TickCount {
   pub count: u32
}



pub async fn run(context: SteadyContext
        ,ticks_rx: SteadyRx<Tick>
        ,tick_counts_tx: SteadyTx<TickCount>) -> Result<(),Box<dyn Error>> {

    let _cli_args = context.args::<Args>();

    let mut monitor =  into_monitor!(context, [ticks_rx],[tick_counts_tx]);

    let mut ticks_rx = ticks_rx.lock().await;
    let mut tick_counts_tx = tick_counts_tx.lock().await;
    let mut buffer = [Tick::default(); 1000];

    while monitor.is_running(&mut || ticks_rx.is_empty() && ticks_rx.is_closed() && tick_counts_tx.mark_closed()) {

         let _clean = wait_for_all!(monitor.wait_vacant_units(&mut tick_counts_tx,1)
                                   ).await;

         let count = monitor.try_peek_slice(&mut ticks_rx, &mut buffer);
         if count>0 {
             let max_count = TickCount { count: buffer[count - 1].value };

             let _ = monitor.try_send(&mut tick_counts_tx, max_count);//.expect("internal error");

             for _n in 0..count {
                 let _ = monitor.try_take(&mut ticks_rx); //TODO: add a better method for this.
             }

             monitor.relay_stats_smartly();
         }
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