
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

#[cfg(not(test))]
pub async fn run<const TICKS_TX_GIRTH:usize,>(context: SteadyContext
                                                            ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, ticks_tx).await
}

#[cfg(test)]
pub async fn run<const TICKS_TX_GIRTH:usize,>(context: SteadyContext
                                              ,tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let mut monitor = into_monitor!(context, [], tx);
    if let Some(responder) = monitor.sidechannel_responder() {

        let mut tx = tx.lock().await;

        while monitor.is_running(&mut || tx.mark_closed() ) {
            let _responder = responder.respond_with(|message| {
                let msg: &Tick = message.downcast_ref::<Tick>().expect("error casting");
                match monitor.try_send(&mut tx[0], msg.clone()) {
                    Ok(()) => Box::new("ok".to_string()),
                    Err(m) => Box::new(m),
                }
            }).await;
            monitor.relay_stats_smartly();
        }
    }
    Ok(())
}

#[allow(unused)]
const BUFFER_SIZE:usize = 1000;

//tag that it is ok that this is never called
#[allow(unused)]
async fn internal_behavior<const TICKS_TX_GIRTH:usize,>(context: SteadyContext
        ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let _cli_args = context.args::<Args>();
    let mut monitor =  into_monitor!(context, [],ticks_tx);

    let mut ticks_tx = ticks_tx.lock().await;
    let batch = ticks_tx.capacity()/8;
    let mut buffers:[Tick; BUFFER_SIZE] = [Tick { value: 0 }; BUFFER_SIZE];

    let mut count: u128 = 0;
    while monitor.is_running(&mut || ticks_tx.mark_closed()) {
         let _clean = wait_for_all!(monitor.wait_shutdown_or_vacant_units_bundle(&mut ticks_tx, batch, TICKS_TX_GIRTH)    );
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


#[cfg(test)]
pub(crate) mod actor_tests {
    use async_std::test;
    // use std::time::Duration;
    // use futures_timer::Delay;
    // use steady_state::*;
    // use crate::actor::tick_generator::{BUFFER_SIZE, internal_behavior};

    #[test]
    pub(crate) async fn test_simple_process() {
        // // build test graph, the input and output channels and our actor
        // let mut graph = GraphBuilder::for_testing().build(()); 
        // let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
        //     .with_capacity(BUFFER_SIZE)
        //     .build_as_bundle::<_, 3>();
        // graph.actor_builder()
        //     .with_name("UnitTest")
        //     .build_spawn( move |context| internal_behavior(context, ticks_tx_out.clone()) );
        // 
        // // run graph until the actor detects the input is closed
        // let timeout = Duration::from_secs(15);
        // graph.start(); //startup the graph
        // 
        // 
        // graph.request_stop(); //our actor has no input so it immediately stops upon this request
        // graph.block_until_stopped(timeout);
        // 
        // // assert expected results
        // assert_eq!(ticks_rx_out[0].testing_avail_units().await, BUFFER_SIZE);
        // assert_eq!(ticks_rx_out[1].testing_avail_units().await, BUFFER_SIZE);
        // assert_eq!(ticks_rx_out[2].testing_avail_units().await, BUFFER_SIZE);
    
    }
}