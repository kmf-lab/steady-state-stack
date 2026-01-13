
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;
use std::error::Error;

use crate::actor::tick_generator::Tick;

const BATCH:usize = 1000;

pub async fn run(context: SteadyActorShadow
        ,ticks_rx: SteadyRx<Tick>
        ,ticks_tx: SteadyTx<Tick>) -> Result<(),Box<dyn Error>> {
    let actor = context.into_spotlight([&ticks_rx], [&ticks_tx]);
    if actor.use_internal_behavior {
        internal_behavior(actor, ticks_rx, ticks_tx).await
    } else {
        actor.simulated_behavior(sim_runners!(ticks_rx, ticks_tx)).await
    }
}

async fn internal_behavior<C: SteadyActor>(mut actor: C, ticks_rx: SteadyRx<Tick>, ticks_tx: SteadyTx<Tick>) -> Result<(), Box<dyn Error>> {
    let _cli_args = actor.args::<Args>();

    let mut ticks_rx = ticks_rx.lock().await;
    let mut ticks_tx = ticks_tx.lock().await;

    let mut buffer = [Tick::default(); BATCH];

    while actor.is_running(&mut || ticks_rx.is_closed_and_empty() && ticks_tx.mark_closed()) {
        let _clean = await_for_all!(
                                    actor.wait_avail(&mut ticks_rx,BATCH),
                                    actor.wait_vacant(&mut ticks_tx,BATCH)
                                   );

        let slice = actor.peek_slice(&mut ticks_rx);
        let count = slice.copy_into_slice(&mut buffer).item_count();

        //do something
        actor.send_slice(&mut ticks_tx, &buffer[0..count]);
        let _ = actor.take_slice(&mut ticks_rx, &mut buffer[0..count]);

        actor.relay_stats_smartly();
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
    fn test_tick_durable_relay() -> Result<(), Box<dyn Error>> {
        //build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing().build(());
        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
            .with_capacity(BATCH).build_channel();
        let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
            .with_capacity(BATCH).build_channel();
        graph.actor_builder()
            .with_name("UnitTest")
            .build( move |context| internal_behavior(context, ticks_rx_in.clone(), ticks_tx_out.clone()), SoloAct );

        //add test data to the input channels

        //run graph until the actor detects when the input is closed
        //we run this before sending data to the input channels so we can cover both branches
        graph.start(); //startup the graph
        sleep(Duration::from_millis(3));  //wait for actor to start
        let test_data:Vec<Tick> = (0..BATCH).map(|i| Tick { value: i as u128 }).collect();
        ticks_tx_in.testing_send_all(test_data, false);

        graph.request_shutdown(); //let all actors close when inputs are closed

        ticks_tx_in.testing_close();
        graph.block_until_stopped(Duration::from_secs(240))?;

        assert_steady_rx_eq_count!(&ticks_rx_out,BATCH);
        Ok(())
    }
}
