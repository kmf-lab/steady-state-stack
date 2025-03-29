
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;
use std::error::Error;

use crate::actor::tick_generator::Tick;

const BATCH:usize = 1000;

pub async fn run(context: SteadyContext
        ,ticks_rx: SteadyRx<Tick>
        ,ticks_tx: SteadyTx<Tick>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, ticks_rx, ticks_tx).await
}

async fn internal_behavior(context: SteadyContext, ticks_rx: SteadyRx<Tick>, ticks_tx: SteadyTx<Tick>) -> Result<(), Box<dyn Error>> {
    let _cli_args = context.args::<Args>();

    let mut monitor = context.into_monitor( [&ticks_rx], [&ticks_tx]);

    let mut ticks_rx = ticks_rx.lock().await;
    let mut ticks_tx = ticks_tx.lock().await;

    let mut buffer = [Tick::default(); BATCH];

    while monitor.is_running(&mut || ticks_rx.is_closed_and_empty() && ticks_tx.mark_closed()) {
        let _clean = await_for_all!(
                                    monitor.wait_avail(&mut ticks_rx,BATCH),
                                    monitor.wait_vacant(&mut ticks_tx,BATCH)
                                   );

        let count = monitor.try_peek_slice(&mut ticks_rx, &mut buffer);
        //do something
        monitor.send_slice_until_full(&mut ticks_tx, &buffer[0..count]);
        let _ = monitor.take_slice(&mut ticks_rx, &mut buffer[0..count]);

        monitor.relay_stats_smartly();
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod hd_actor_tests {
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::*;
    use crate::actor::tick_generator::Tick;
    use crate::actor::tick_relay::{internal_behavior, BATCH};

    #[test]
    fn test_tick_durable_relay() {
        //build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing().build(());
        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
            .with_capacity(BATCH).build();
        let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
            .with_capacity(BATCH).build();
        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context| internal_behavior(context, ticks_rx_in.clone(), ticks_tx_out.clone()) );

        //add test data to the input channels

        //run graph until the actor detects when the input is closed
        //we run this before sending data to the input channels so we can cover both branches
        graph.start(); //startup the graph
        sleep(Duration::from_millis(3));  //wait for actor to start
        let test_data:Vec<Tick> = (0..BATCH).map(|i| Tick { value: i as u128 }).collect();
        ticks_tx_in.testing_send_all(test_data, false);

        graph.request_stop(); //let all actors close when inputs are closed

        ticks_tx_in.testing_close();
        graph.block_until_stopped(Duration::from_secs(240));

        assert_steady_rx_eq_count!(&ticks_rx_out,BATCH);
    }
}