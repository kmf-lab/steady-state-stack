
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use steady_state::monitor::LocalMonitor;
use crate::Args;
use futures::join;
use futures::select;

use std::error::Error;


#[derive(Default)]
pub(crate) struct Tick {
   //TODO: : add your fields here
}




//if no internal state is required (recommended) feel free to remove this.
#[derive(Default)]
struct TickgeneratorInternalState {
     
     
     
}
impl TickgeneratorInternalState {
    fn new(cli_args: &Args) -> Self {
        Self {
           ////TODO: : add custom arg based init here
           ..Default::default()
        }
    }
}



pub async fn run<const TICKS_TX_GIRTH:usize,>(context: SteadyContext
        ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let cli_args = context.args::<Args>();
    let mut state = if let Some(args) = cli_args {
        TickgeneratorInternalState::new(args)
    } else {
        TickgeneratorInternalState::default()
    };

    let mut monitor =  into_monitor!(context, [],[
                        ticks_tx[0],
                        ticks_tx[1],
                        ticks_tx[2]]
                           );

 
    let mut ticks_tx = ticks_tx.lock().await;

    while monitor.is_running(&mut ||ticks_tx.mark_closed()) {

         let _clean = wait_for_all!(monitor.wait_periodic(Duration::from_millis(1000))    ).await;


     process_once(&mut monitor, &mut state
         , &mut ticks_tx).await;

     monitor.relay_stats_smartly();

    }
    Ok(())
}

async fn process_once<const R: usize, const T: usize>(monitor: & mut LocalMonitor<R,T>
                          , state: &mut TickgeneratorInternalState, ticks_tx: &mut TxBundle<'_, Tick>
                             ) {

    
    ////TODO: : put your implementation here

}

/*

[cfg(test)]
pub async fn run<const TICKS_TX_GIRTH:usize,>(context: SteadyContext
        ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {

}

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
         ////TODO: : add assignments

        process_once(&mut monitor, &mut state
                 , &mut ticks_tx).await;


    }
}
*/