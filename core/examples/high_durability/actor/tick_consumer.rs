
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
    internal_behavior(context, ticks_rx, tick_counts_tx).await
}

async fn internal_behavior(context: SteadyActorShadow, ticks_rx: SteadyRx<Tick>, tick_counts_tx: SteadyTx<TickCount>) -> Result<(), Box<dyn Error>> {
    let _cli_args = context.args::<Args>();

    // create the monitor for doing all channel work
    let mut actor = context.into_spotlight([&ticks_rx], [&tick_counts_tx]);

    // lock the channels for use in this instance
    let mut ticks_rx = ticks_rx.lock().await;
    let mut tick_counts_tx = tick_counts_tx.lock().await;
    let mut buffer = [Tick::default(); 1000];

    // start 'is running' loop so we detect shutdown an exit clean
    while actor.is_running(&mut || ticks_rx.is_closed_and_empty() && tick_counts_tx.mark_closed()) {
        
        // async on any required conditions to ensure we are not spinning
        let _clean = await_for_all!(
                                    actor.wait_avail(&mut ticks_rx,100),
                                    actor.wait_vacant(&mut tick_counts_tx,1)
                                   );

        let slice = actor.peek_slice(&mut ticks_rx);
        let count = slice.copy_into_slice(&mut buffer).item_count();
        if count > 0 {
            let max_count = TickCount { count: buffer[count - 1].value };
            let _ = actor.try_send(&mut tick_counts_tx, max_count);
            actor.take_slice(&mut ticks_rx, &mut buffer[0..count]);
        }
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod hd_actor_tests {
    use std::thread::sleep;
    #[allow(unused_imports)]
    use log::*;
    use steady_state::*;
    use crate::actor::tick_consumer::{internal_behavior};
    use crate::actor::tick_generator::Tick;

    const WAIT_AVAIL: usize = 250;
    #[test]
    fn test_tick_durablity_consumer() -> Result<(), Box<dyn Error>> {
        //build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing().build(());

        let (ticks_tx_in, ticks_rx_in) = graph.channel_builder()
            .with_capacity(WAIT_AVAIL).build_channel();
        let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
            .with_capacity(WAIT_AVAIL).build_channel();

        graph.actor_builder()
            .with_name("UnitTest")
            .build( move |context| internal_behavior(context, ticks_rx_in.clone(), ticks_tx_out.clone()), SoloAct );

        graph.start_with_timeout(Duration::from_secs(2)); //startup the graph

        //add test data to the input channels
        let test_data:Vec<Tick> = (0..WAIT_AVAIL).map(|i| Tick { value: (i+1) as u128 }).collect();
        ticks_tx_in.testing_send_all(test_data,true);

        sleep(Duration::from_millis(300));
        graph.request_shutdown();

        graph.block_until_stopped(Duration::from_secs(12))?;
        sleep(Duration::from_millis(20));

        let expected = 0;
        assert_steady_rx_gt_count!(&ticks_rx_out,expected);
        Ok(())
    }
}