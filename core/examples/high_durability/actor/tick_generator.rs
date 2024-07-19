
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;



use std::error::Error;


#[derive(Default,Clone,Copy)]
pub struct Tick {
  pub value: u128
}



pub async fn run<const TICKS_TX_GIRTH:usize,>(context: SteadyContext
        ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let _cli_args = context.args::<Args>();

    let mut monitor =  into_monitor!(context, [],ticks_tx);

 
    let mut ticks_tx = ticks_tx.lock().await;
    let batch = ticks_tx.capacity()/8;

    const BUFFER_SIZE:usize = 1000;
    let mut buffers:[Tick; BUFFER_SIZE] = [Tick { value: 0 }; BUFFER_SIZE];


    let mut count: u128 = 0;
    while monitor.is_running(&mut || ticks_tx.mark_closed()) {

         let _clean = wait_for_all!(monitor.wait_vacant_units_bundle(&mut ticks_tx, batch, TICKS_TX_GIRTH)    )
             .await;

         for i in 0..TICKS_TX_GIRTH {
             let c = ticks_tx[i].vacant_units().min(BUFFER_SIZE);

             for n in 0..c {
                 count = count + 1;
                 buffers[n] = Tick { value: count };
             }
             monitor.send_slice_until_full(&mut ticks_tx[i], &buffers[..c]);

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
         let (ticks_tx,ticks_rx_extern) = graph.channel_builder().with_capacity(8).build();
         let mock_context = graph.new_test_monitor("mock");
         let mut mock_monitor = into_monitor!(mock_context, [], []);
         let mut ticks_tx = ticks_tx.lock().await;
         let mut ticks_rx_extern = ticks_rx_extern.lock().await;



    }
}

 */