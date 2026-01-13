#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use std::error::Error;
use crate::actor::div_by_3_producer::NumberMessage;
use crate::actor::fizz_buzz_processor;

pub async fn run(context: SteadyActorShadow, numbers_tx: SteadyTx<NumberMessage>) -> Result<(),Box<dyn Error>> {
    let actor = context.into_spotlight([], [&numbers_tx]);
    if actor.use_internal_behavior {
        internal_behavior(actor, numbers_tx).await
    } else {
        actor.simulated_behavior(sim_runners!(numbers_tx)).await
    }
}


const BATCH_SIZE: usize = 4000;
const STEP_SIZE: u64 = 5;


async fn internal_behavior<A: SteadyActor>(mut actor: A
                                           , numbers_tx: SteadyTx<NumberMessage>) -> Result<(),Box<dyn Error>> {

    let mut numbers_tx = numbers_tx.lock().await;

    let mut buffer:[NumberMessage; BATCH_SIZE] = [NumberMessage::default(); BATCH_SIZE];
    let mut index:u64 = 0;

    while actor.is_running(&mut || i!(numbers_tx.mark_closed())) {
        let _clean = await_for_all!(actor.wait_vacant(&mut numbers_tx, BATCH_SIZE>>1));

        let mut i = 0;
        let limit = BATCH_SIZE.min(actor.vacant_units(&mut numbers_tx));
        loop {
            index += STEP_SIZE;
            buffer[i] = NumberMessage{value:index};
            i += 1;
            if i >= limit || index == fizz_buzz_processor::STOP_VALUE {
                if index >= fizz_buzz_processor::STOP_VALUE {
                    actor.request_shutdown().await;
                }
                break;
            }
        }
        let _sent_count = actor.send_slice(&mut numbers_tx, &buffer[0..i]);
    }
    actor.request_shutdown().await;
    Ok(())
}




#[cfg(test)]
pub(crate) mod tests {
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::*;
    use super::*;

    #[test]
    fn test_div_by_5_producer() -> Result<(), Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(());

        let (numbers_tx, test_numbers_rx) = graph.channel_builder()
                                             .with_capacity(50000).build_channel();

        graph.actor_builder()
            .with_name("UnitTest")
            .build(move |context|
                internal_behavior(context, numbers_tx.clone()), SoloAct
            );

        graph.start(); //startup the graph
        sleep(Duration::from_millis(20));
        graph.request_shutdown(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(1))?;

        let expected = vec!(NumberMessage { value: 5 }, NumberMessage { value: 10 }, NumberMessage { value: 15 });
        assert_steady_rx_eq_take!(&test_numbers_rx,expected);
        Ok(())
    }
}
