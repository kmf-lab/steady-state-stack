
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
   pub count: u128
}

const BATCH: usize = 2000;
const WAIT_AVAIL: usize = 250;

pub async fn run(context: SteadyContext
        ,ticks_rx: SteadyRx<Tick>
        ,tick_counts_tx: SteadyTx<TickCount>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, ticks_rx, tick_counts_tx).await
}

async fn internal_behavior(context: SteadyContext, ticks_rx: SteadyRx<Tick>, tick_counts_tx: SteadyTx<TickCount>) -> Result<(), Box<dyn Error>> {
    let _cli_args = context.args::<Args>();

    let mut monitor = into_monitor!(context, [ticks_rx],[tick_counts_tx]);

    let mut ticks_rx = ticks_rx.lock().await;
    let mut tick_counts_tx = tick_counts_tx.lock().await;
    let mut buffer = [Tick::default(); 1000];

    while monitor.is_running(&mut || ticks_rx.is_closed_and_empty() && tick_counts_tx.mark_closed()) {
        let _clean = wait_for_all!(
                                    monitor.wait_avail_units(&mut ticks_rx,100),
                                    monitor.wait_vacant_units(&mut tick_counts_tx,1)
                                   ).await;

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
mod actor_tests {
    use std::time::Duration;
    use async_std::test;
    use steady_state::*;
    use crate::actor::tick_consumer::{internal_behavior, WAIT_AVAIL};
    use crate::actor::tick_generator::Tick;

    #[test]
    async fn test_simple_process() {
        //1. build test graph, the input and output channels and our actor
        let mut graph = Graph::new_test(());
        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
            .with_capacity(WAIT_AVAIL).build();
        let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
            .with_capacity(WAIT_AVAIL).build();
        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context| internal_behavior(context, ticks_rx_in.clone(), ticks_tx_out.clone()) );

        //2. add test data to the input channels
        let test_data:Vec<Tick> = (0..WAIT_AVAIL).map(|i| Tick { value: i as u128 }).collect();
        ticks_tx_in.testing_send(test_data, true).await;

        //3. run graph until the actor detects the input is closed
        graph.start_as_data_driven(Duration::from_secs(240));

        //4. assert expected results
        assert_eq!(ticks_rx_out.testing_avail_units().await, 1);
    }
}