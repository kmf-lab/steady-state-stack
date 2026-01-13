
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


pub async fn run(context: SteadyActorShadow
        ,ticks_rx: SteadyRx<Tick>
        ,tick_counts_tx: SteadyTx<TickCount>) -> Result<(),Box<dyn Error>> {
    let actor = context.into_spotlight([&ticks_rx], [&tick_counts_tx]);
    if actor.use_internal_behavior {
        internal_behavior(actor, ticks_rx, tick_counts_tx).await
    } else {
        actor.simulated_behavior(sim_runners!(ticks_rx, tick_counts_tx)).await
    }
}

const BATCH: usize = 2000;
const WAIT_AVAIL: usize = 250;

async fn internal_behavior<C: SteadyActor>(mut actor: C, rx: SteadyRx<Tick>, tx: SteadyTx<TickCount>) -> Result<(), Box<dyn Error>> {
    let _cli_args = actor.args::<Args>();

    let mut ticks_rx = rx.lock().await;
    let mut tick_counts_tx = tx.lock().await;
    let mut buffer = [Tick::default(); BATCH];

    while actor.is_running(&mut || ticks_rx.is_closed_and_empty() && tick_counts_tx.mark_closed()) {
        let _clean = await_for_all!(
                                   actor.wait_avail(&mut ticks_rx,WAIT_AVAIL),
                                   actor.wait_vacant(&mut tick_counts_tx,1)
                                   );

        let count = actor.take_slice(&mut ticks_rx, &mut buffer).item_count();
        if count > 0 {
            let max_count = TickCount { count: buffer[count - 1].value };
            let _ = actor.try_send(&mut tick_counts_tx, max_count);
            actor.relay_stats_smartly();
        }
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod hp_actor_tests {
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::*;
    use crate::actor::tick_consumer::{internal_behavior, WAIT_AVAIL};
    use crate::actor::tick_generator::Tick;

    #[test]
    fn test_tick_consumer() -> Result<(), Box<dyn Error>> {
        // build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing()
                                      .build(());
        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
            .with_capacity(WAIT_AVAIL)
            .build_channel();
        let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
            .with_capacity(WAIT_AVAIL)
            .build_channel();
        graph.actor_builder()
            .with_name("UnitTest")
            .build( move |context| internal_behavior(context, ticks_rx_in.clone(), ticks_tx_out.clone()), SoloAct );

        graph.start();
        graph.request_shutdown();

        let test_data:Vec<Tick> = (0..WAIT_AVAIL).map(|i| Tick { value: i as u128 }).collect();
        
        ticks_tx_in.testing_send_all(test_data,true);
        sleep(Duration::from_millis(50));

        graph.block_until_stopped(Duration::from_secs(240))?;
        let expected = 0;
        assert_steady_rx_gt_count!(&ticks_rx_out,expected);
        Ok(())
    }
}
