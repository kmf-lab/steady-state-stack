
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;
use std::error::Error;
use crate::actor::tick_generator::Tick;


#[derive(Default,Clone,Copy,Debug,PartialEq,Ord,PartialOrd,Eq)]
pub struct TickCount {
   pub count: u128
}

const WAIT_AVAIL: usize = 250;

pub async fn run(context: SteadyContext
        ,ticks_rx: SteadyRx<Tick>
        ,tick_counts_tx: SteadyTx<TickCount>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, ticks_rx, tick_counts_tx).await
}

async fn internal_behavior(context: SteadyContext, ticks_rx: SteadyRx<Tick>, tick_counts_tx: SteadyTx<TickCount>) -> Result<(), Box<dyn Error>> {
    let _cli_args = context.args::<Args>();

    // create the monitor for doing all channel work
    let mut monitor = context.into_monitor( [&ticks_rx],[&tick_counts_tx]);

    // lock the channels for use in this instance
    let mut ticks_rx = ticks_rx.lock().await;
    let mut tick_counts_tx = tick_counts_tx.lock().await;
    let mut buffer = [Tick::default(); 1000];

    // start 'is running' loop so we detect shutdown an exit clean
    while monitor.is_running(&mut || ticks_rx.is_closed_and_empty() && tick_counts_tx.mark_closed()) {
        
        // async on any required conditions to ensure we are not spinning
        let _clean = await_for_all!(
                                    monitor.wait_avail(&mut ticks_rx,100),
                                    monitor.wait_vacant(&mut tick_counts_tx,1)
                                   );

        let count = monitor.try_peek_slice(&mut ticks_rx, &mut buffer);
        if count > 0 {
            let max_count = TickCount { count: buffer[count - 1].value };
            let _ = monitor.try_send(&mut tick_counts_tx, max_count);
            monitor.take_slice(&mut ticks_rx, &mut buffer[0..count]);
            monitor.relay_stats_smartly();
        }
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod hd_actor_tests {
    use std::time::Duration;
    use async_std::test;
    #[allow(unused_imports)]
    use log::*;
    use steady_state::*;
    use crate::actor::tick_consumer::{internal_behavior, WAIT_AVAIL};
    use crate::actor::tick_generator::Tick;

    #[test]
    pub(crate) async fn test_simple_process() {
        //build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing().build(());
        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
            .with_capacity(WAIT_AVAIL).build();
        let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
            .with_capacity(WAIT_AVAIL).build();
        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context| internal_behavior(context, ticks_rx_in.clone(), ticks_tx_out.clone()) );


        graph.start_with_timeout(Duration::from_secs(7)); //startup the graph

        //add test data to the input channels
        let test_data:Vec<Tick> = (0..WAIT_AVAIL).map(|i| Tick { value: (i+1) as u128 }).collect();
        ticks_tx_in.testing_send_all(test_data,true).await;
        
        graph.request_stop();


        assert_eq!(true, graph.block_until_stopped(Duration::from_secs(12)));
        assert_eq!(true, ticks_rx_out.testing_avail_units().await>0);
        assert_eq!(WAIT_AVAIL as u128, ticks_rx_out.testing_take().await.last().expect("count").count);

    }
}