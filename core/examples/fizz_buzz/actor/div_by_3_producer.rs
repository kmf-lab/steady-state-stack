
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use std::error::Error;
use crate::actor::fizz_buzz_processor;

#[derive(Default, Clone, Copy, PartialOrd, PartialEq, Debug)]
pub(crate) struct NumberMessage {
   pub(crate) value: u64
}


pub async fn run(context: SteadyContext, numbers_tx: SteadyTx<NumberMessage>) -> Result<(),Box<dyn Error>> {
    let cmd = context.into_monitor( [],[&numbers_tx]);
    if cfg!(not(test)) {
        internal_behavior(cmd,numbers_tx).await
    } else {
        cmd.simulated_behavior(vec!(&SimTx(numbers_tx))).await
    }
  }


const BATCH_SIZE: usize = 4000;
const STEP_SIZE: u64 = 3;

async fn internal_behavior<C: SteadyCommander>(mut cmd: C
        ,numbers_tx: SteadyTx<NumberMessage>) -> Result<(),Box<dyn Error>> {

    let mut numbers_tx = numbers_tx.lock().await;

    let mut buffer:[NumberMessage; BATCH_SIZE] = [NumberMessage::default(); BATCH_SIZE];
    let mut index:u64 = 0;


    while cmd.is_running(&mut || i!(numbers_tx.mark_closed())) {
        let _clean = await_for_all!(cmd.wait_vacant(&mut numbers_tx, BATCH_SIZE>>1));

        let mut i = 0;
        let limit = BATCH_SIZE.min(cmd.vacant_units(&mut numbers_tx));
        loop {
                        index = index + STEP_SIZE;
                        buffer[i] = NumberMessage{value:index};
                        i = i + 1;
                        if i >= limit || index == fizz_buzz_processor::STOP_VALUE {
                            if index >= fizz_buzz_processor::STOP_VALUE {
                                cmd.request_graph_stop().await;
                            }
                            break;
                        }
        }
        let _sent_count = cmd.send_slice_until_full(&mut numbers_tx, &buffer[0..i]); //TOOD: zero length? test?
    }
    cmd.request_graph_stop().await;
    Ok(())
}



#[cfg(test)]
pub(crate) mod tests {
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::*;
    use super::*;

    #[test]
    fn test_div_by_3_producer() -> Result<(), Box<dyn Error>>{
       let mut graph = GraphBuilder::for_testing().build(());

       let (numbers_tx, test_numbers_rx) = graph.channel_builder()
                   .with_capacity(50000)
                   .build_channel();

        graph.actor_builder()
                    .with_name("UnitTest")
                    .build_spawn( move |context|
                            internal_behavior(context, numbers_tx.clone())
                     );

        graph.start(); //startup the graph
        sleep(Duration::from_millis(40));
        graph.request_stop(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(1))?;

        let expected = vec!(NumberMessage { value: 3 }, NumberMessage { value: 6 }, NumberMessage { value: 9 });
        assert_steady_rx_eq_take!(&test_numbers_rx,expected);
        Ok(())
    }


}