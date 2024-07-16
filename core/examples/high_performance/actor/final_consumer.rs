
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use steady_state::monitor::LocalMonitor;
use crate::Args;

use std::error::Error;
use crate::actor::tick_consumer::TickCount;






//if no internal state is required (recommended) feel free to remove this.
#[derive(Default)]
struct FinalconsumerInternalState {
     
     
}
impl FinalconsumerInternalState {
    fn new(cli_args: &Args) -> Self {
        Self {
           ////TODO: : add custom arg based init here
           ..Default::default()
        }
    }
}



pub async fn run<const TICK_COUNTS_RX_GIRTH:usize,>(context: SteadyContext
        ,tick_counts_rx: SteadyRxBundle<TickCount, TICK_COUNTS_RX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let cli_args = context.args::<Args>();
    let mut state = if let Some(args) = cli_args {
        FinalconsumerInternalState::new(args)
    } else {
        FinalconsumerInternalState::default()
    };

    let mut monitor =  into_monitor!(context, [
                        tick_counts_rx[0],
                        tick_counts_rx[1],
                        tick_counts_rx[2]],[]
                           );

 let mut tick_counts_rx = tick_counts_rx.lock().await;
 

    while monitor.is_running(&mut ||
       tick_counts_rx.is_closed_and_empty() ) {

         let _clean = wait_for_all!(monitor.wait_avail_units_bundle(&mut tick_counts_rx,1,1)    ).await;


     process_once(&mut monitor, &mut state
         , &mut tick_counts_rx).await;

     monitor.relay_stats_smartly();

    }
    Ok(())
}

async fn process_once<const R: usize, const T: usize>(monitor: & mut LocalMonitor<R,T>
                          , state: &mut FinalconsumerInternalState, tick_counts_rx: &mut RxBundle<'_, TickCount>
                             ) {

    //trythis:  monitor.take_slice(tick_counts_rx);

    ////TODO: : put your implementation here

}
