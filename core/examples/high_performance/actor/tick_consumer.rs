
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
use crate::actor::tick_generator::Tick;


#[derive(Default,Clone,Copy)]
pub(crate) struct TickCount {
   //TODO: : add your fields here
}




//if no internal state is required (recommended) feel free to remove this.
#[derive(Default)]
struct Tickconsumer3InternalState {
     
     
     
}
impl Tickconsumer3InternalState {
    fn new(cli_args: &Args) -> Self {
        Self {
           ////TODO: : add custom arg based init here
           ..Default::default()
        }
    }
}



pub async fn run(context: SteadyContext
        ,ticks_rx: SteadyRx<Tick>
        ,tick_counts_tx: SteadyTx<TickCount>) -> Result<(),Box<dyn Error>> {

    let cli_args = context.args::<Args>();
    let mut state = if let Some(args) = cli_args {
        Tickconsumer3InternalState::new(args)
    } else {
        Tickconsumer3InternalState::default()
    };

    let mut monitor =  into_monitor!(context, [
                        ticks_rx],[
                        tick_counts_tx]
                           );

 let mut ticks_rx = ticks_rx.lock().await;
 
    let mut tick_counts_tx = tick_counts_tx.lock().await;

    while monitor.is_running(&mut ||
    ticks_rx.is_closed_and_empty() && tick_counts_tx.mark_closed()) {

         let _clean = wait_for_all!(monitor.wait_periodic(Duration::from_millis(1000))    ).await;


     process_once(&mut monitor, &mut state
         , &mut ticks_rx
         , &mut tick_counts_tx).await;

     monitor.relay_stats_smartly();

    }
    Ok(())
}

async fn process_once<const R: usize, const T: usize>(monitor: & mut LocalMonitor<R,T>
                          , state: &mut Tickconsumer3InternalState, ticks_rx: &mut Rx<Tick>
                             , tick_counts_tx: &mut Tx<TickCount>
                             ) {

    //trythis:  monitor.try_take(ticks_rx);
//trythis:  monitor.try_send(tick_counts_tx, copy );

    ////TODO: : put your implementation here

}

/*
#[cfg(test)]
pub async fn run(context: SteadyContext
        ,ticks_rx: SteadyRx<Tick>
        ,tick_counts_tx: SteadyTx<TickCount>) -> Result<(),Box<dyn Error>> {

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
         let (ticks_tx_extern, ticks_rx) = graph.channel_builder().with_capacity(8).build();
         
         let (tick_counts_tx,tick_counts_rx_extern) = graph.channel_builder().with_capacity(8).build();
         let mock_context = graph.new_test_monitor("mock");
         let mut mock_monitor = into_monitor!(mock_context, [], []);
         let mut ticks_tx_extern = ticks_tx_extern.lock().await;
         let mut ticks_rx = ticks_rx.lock().await;
         
         let mut tick_counts_tx = tick_counts_tx.lock().await;
         let mut tick_counts_rx_extern = tick_counts_rx_extern.lock().await;
         ////TODO: : add assignments

        process_once(&mut monitor, &mut state
                 , &mut ticks_rx
                 , &mut tick_counts_tx).await;


    }
}

 */