
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;
use std::error::Error;
use steady_state::commander::SteadyCommander;
use crate::actor::tick_generator::Tick;

const BATCH:usize = 2000;

pub async fn run(context: SteadyContext
                    ,ticks_rx: SteadyRx<Tick>
                    ,ticks_tx: SteadyTx<Tick>) -> Result<(),Box<dyn Error>> {
    internal_behavior(into_monitor!(context, [ticks_rx],[ticks_tx]), ticks_rx, ticks_tx).await
}

async fn internal_behavior<CMD: SteadyCommander>(mut context: CMD, ticks_rx: SteadyRx<Tick>, ticks_tx: SteadyTx<Tick>) -> Result<(), Box<dyn Error>> {
    let _cli_args = context.args::<Args>();
    let mut ticks_rx_lock = ticks_rx.lock().await;
    let mut ticks_tx_lock = ticks_tx.lock().await;
    let mut buffer = [Tick::default(); BATCH];

    while context.is_running(&mut || ticks_rx_lock.is_closed_and_empty() && ticks_tx_lock.mark_closed()) {
        let _clean = await_for_all!(
                                context.wait_shutdown_or_avail_units(&mut ticks_rx_lock,BATCH),
                                context.wait_shutdown_or_vacant_units(&mut ticks_tx_lock,BATCH)
                               );
        let count = context.take_slice(&mut ticks_rx_lock, &mut buffer);
        context.send_slice_until_full(&mut ticks_tx_lock, &buffer[0..count]);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod actor_tests {
    use std::time::Duration;
    use futures_timer::Delay;
    use steady_state::*;
    use crate::actor::tick_generator::Tick;
    use crate::actor::tick_relay::{BATCH, internal_behavior};

    #[async_std::test]
    pub(crate) async fn test_simple_process() {
        // build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing()
                         .build(());

        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
                                              .with_capacity(BATCH*3).build();
        let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
                                               .with_capacity(BATCH*3).build();


        graph.actor_builder()
             .with_name("UnitTest")
             .build_spawn( move |context| internal_behavior(context, ticks_rx_in.clone(), ticks_tx_out.clone()) );

        let test_data:Vec<Tick> = (0..BATCH).map(|i| Tick { value: i as u128 }).collect();
        graph.start();
        Delay::new(Duration::from_millis(5)).await; //if too long telemetry will back up

        ticks_tx_in.testing_send_all(test_data,true).await;

        graph.request_stop();
        assert_eq!(true,graph.block_until_stopped(Duration::from_secs(2)));

        assert_eq!(ticks_rx_out.testing_avail_units().await, BATCH);
    }
}
