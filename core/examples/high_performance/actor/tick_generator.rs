
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;

use std::error::Error;

#[derive(Default, Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tick {
  pub value: u128
}

pub async fn run<const TICKS_TX_GIRTH:usize>(context: SteadyContext
                                                            ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {
    let cmd = context.into_monitor([], ticks_tx.meta_data());
    if cfg!(not(test)) {
        internal_behavior(cmd, ticks_tx).await
    } else {
        cmd.simulated_behavior(vec!(&TestEcho(ticks_tx[0].clone()))).await
    }
}

const BUFFER_SIZE:usize = 2000;

async fn internal_behavior<const TICKS_TX_GIRTH:usize,C:SteadyCommander>(mut cmd: C
        ,ticks_tx: SteadyTxBundle<Tick, TICKS_TX_GIRTH>) -> Result<(),Box<dyn Error>> {

    let _cli_args = cmd.args::<Args>();

    let mut ticks_tx = ticks_tx.lock().await;
    let batch = ticks_tx.capacity()/4;
    let mut buffers:[Tick; BUFFER_SIZE] = [Tick { value: 0 }; BUFFER_SIZE];

    let mut count: u128 = 0;
    while cmd.is_running(&mut || ticks_tx.mark_closed()) {
         let _clean = await_for_all!(cmd.wait_vacant_bundle(&mut ticks_tx, batch, TICKS_TX_GIRTH)    );
         for i in 0..TICKS_TX_GIRTH {
             
             let c = ticks_tx[i].vacant_units().min(BUFFER_SIZE);
             for n in 0..c {
                 count = count + 1;
                 buffers[n] = Tick { value: count };
             }
             cmd.send_slice_until_full(&mut ticks_tx[i], &buffers[..c]);
             
         }
        cmd.relay_stats_smartly();
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod actor_tests {
    use std::time::Duration;
    use futures_timer::Delay;
    use steady_state::*;
    use super::*;

    #[async_std::test]
    pub(crate) async fn test_simple_process() {
        let mut graph = GraphBuilder::for_testing().build(());

        let (ticks_tx_out,ticks_rx_out) = graph.channel_builder()
            .with_capacity(BUFFER_SIZE)
            .build_as_bundle::<_, 3>();

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context| internal_behavior(context, ticks_tx_out.clone()) );

        graph.start(); //startup the graph
  
        Delay::new(Duration::from_millis(40)).await; //if too long telemetry will back up
        
        graph.request_stop(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(15));

        assert_eq!(ticks_rx_out[0].testing_avail_units().await, BUFFER_SIZE);
        assert_eq!(ticks_rx_out[1].testing_avail_units().await, BUFFER_SIZE);
        assert_eq!(ticks_rx_out[2].testing_avail_units().await, BUFFER_SIZE);

    }
}