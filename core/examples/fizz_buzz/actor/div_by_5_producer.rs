
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use std::error::Error;
use crate::actor::div_by_3_producer::NumberMessage;
use crate::actor::fizz_buzz_processor;

#[cfg(not(test))]
pub async fn run(context: SteadyContext
        ,numbers_tx: SteadyTx<NumberMessage>) -> Result<(),Box<dyn Error>> {
  internal_behavior(context.into_monitor([],[&numbers_tx]),numbers_tx).await
}

const BATCH_SIZE: usize = 4000;
const STEP_SIZE: u64 = 5;


async fn internal_behavior<C:SteadyCommander>(mut cmd: C
                           ,numbers_tx: SteadyTx<NumberMessage>) -> Result<(),Box<dyn Error>> {

    let mut numbers_tx = numbers_tx.lock().await;

    let mut buffer:[NumberMessage; BATCH_SIZE] = [NumberMessage::default(); BATCH_SIZE];
    let mut index:u64 = 0;

    while cmd.is_running(&mut || index>=fizz_buzz_processor::STOP_VALUE && numbers_tx.mark_closed()) {
        let _clean = await_for_all!(cmd.wait_vacant(&mut numbers_tx, BATCH_SIZE>>1));

        let mut i = 0;
        let limit = BATCH_SIZE.min(cmd.vacant_units(&mut numbers_tx));
        loop {
            index = index + STEP_SIZE;
            buffer[i] = NumberMessage{value:index};
            i = i + 1;
            if i >= limit || index == fizz_buzz_processor::STOP_VALUE {
                if index == fizz_buzz_processor::STOP_VALUE {
                    cmd.request_graph_stop();
                }
                break;
            }
        }
        let _sent_count = cmd.send_slice_until_full(&mut numbers_tx, &buffer[0..i]);
    }
    cmd.request_graph_stop();
    Ok(())
}


#[cfg(test)]
pub async fn run(context: SteadyContext
                 , tx: SteadyTx<NumberMessage>
) -> Result<(),Box<dyn Error>> {
    let mut cmd =  context.into_monitor( [],[&tx]);
    if let Some(responder) = cmd.sidechannel_responder() {
        let mut numbers_tx = tx.lock().await;
        while cmd.is_running(&mut ||numbers_tx.mark_closed()) {
            // in main use graph.sidechannel_director node_call(msg,"DivBy5Producer")
            let _did_echo = responder.echo_responder(&mut cmd,&mut numbers_tx).await;
        }
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;
    use steady_state::*;
    use super::*;
    use futures_timer::Delay;

    #[async_std::test]
    async fn test_simple_process() {
        let mut graph = GraphBuilder::for_testing()
            .with_telemetry_metric_features(false)
            .build(());

        let (numbers_tx, test_numbers_rx) = graph.channel_builder()
                                             .with_capacity(50000).build();

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn(move |context|
                internal_behavior(context, numbers_tx.clone())
            );

        graph.start(); //startup the graph
        Delay::new(Duration::from_millis(2)).await;
        graph.request_stop(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(1));

        let vec = test_numbers_rx.testing_take().await;

        assert_eq!(vec[0].value, 5, "vec: {:?}", vec);
        assert_eq!(vec[1].value, 10, "vec: {:?}", vec);
        assert_eq!(vec[2].value, 15, "vec: {:?}", vec);
    }
}