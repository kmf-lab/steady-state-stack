use std::cmp::{PartialEq, PartialOrd};
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use std::error::Error;
use crate::actor::div_by_3_producer::NumberMessage;

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub(crate) struct ErrorMessage {
   pub(crate) text: String
}

#[derive(Copy, Clone, Default, Debug, PartialOrd, Eq, PartialEq)]
#[repr(u64)] // Pack everything into 8 bytes
pub(crate) enum FizzBuzzMessage {
    #[default]
    FizzBuzz = 15,         // Discriminant is 15
    Fizz = 3,              // Discriminant is 3
    Buzz = 5,              // Discriminant is 5
    Value(u64),            // Store u64 directly, use the fact that FizzBuzz/Fizz/Buzz only occupy small values
}

const BATCH_SIZE: usize = 7000;

#[derive(Copy,Clone)]
pub(crate) struct RuntimeState {
    value: u64,
    i_three: Option<NumberMessage>,
    i_five: Option<NumberMessage>,
    buffer: [FizzBuzzMessage; BATCH_SIZE],
}

impl RuntimeState {
    pub(crate) fn new(value: i32) -> Self {
        RuntimeState {
            value: value as u64,
            i_three: None,
            i_five: None,
            buffer: [FizzBuzzMessage::default(); BATCH_SIZE]
        }
    }
}

pub(crate) const STOP_VALUE: u64        = 1_200_000_000_000;
pub(crate) const PANIC_COUNTDOWN: usize =     6_000_000_000;

pub async fn run<const NUMBERS_RX_GIRTH:usize,>(context: SteadyContext
                                                , numbers_rx: SteadyRxBundle<NumberMessage, NUMBERS_RX_GIRTH>
                                                , fizzbuzz_messages_tx: SteadyTx<FizzBuzzMessage>
                                                , errors_tx: SteadyTx<ErrorMessage>, state: SteadyState<RuntimeState>) -> Result<(),Box<dyn Error>> {

  internal_behavior(context.into_monitor(numbers_rx.meta_data(),[&fizzbuzz_messages_tx, &errors_tx])
                    ,STOP_VALUE,numbers_rx,fizzbuzz_messages_tx,errors_tx, state).await
}


async fn internal_behavior<C: SteadyCommander,const NUMBERS_RX_GIRTH: usize>(
    mut cmd: C,
    stop_value: u64,
    numbers_rx: SteadyRxBundle<NumberMessage, NUMBERS_RX_GIRTH>,
    fizzbuzz_messages_tx: SteadyTx<FizzBuzzMessage>,
    errors_tx: SteadyTx<ErrorMessage>,
    state: SteadyState<RuntimeState>,
) -> Result<(), Box<dyn Error>> {
    let mut state = state.lock(|| RuntimeState::new(1)).await;
        let mut numbers_rx = numbers_rx.lock().await;
        let mut fizzbuzz_messages_tx = fizzbuzz_messages_tx.lock().await;
        let mut errors_tx = errors_tx.lock().await;

        if state.value > 1 {
            let _ = cmd
                .send_async(
                    &mut errors_tx,
                    ErrorMessage {
                        text: format!("at value: {}", state.value),
                    },
                    SendSaturation::AwaitForRoom,
                )
                .await;
        }
        let (threes_rx, fives_rx) = numbers_rx.split_at_mut(1);
        let mut panic_countdown = PANIC_COUNTDOWN as isize;
        let c1 = threes_rx[0].capacity() >> 1;
        let c2 = fives_rx[0].capacity() >> 1;
        let vacant_block = BATCH_SIZE.min(fizzbuzz_messages_tx.capacity());
        while cmd.is_running(&mut || {
                                    state.value == stop_value
                                        && threes_rx[0].is_closed_and_empty()
                                        && fives_rx[0].is_closed_and_empty()
                                        && fizzbuzz_messages_tx.mark_closed()
                                        && errors_tx.mark_closed()
                                }) {
            let _clean = await_for_all!(
                cmd.wait_avail(&mut threes_rx[0], c1),
                cmd.wait_avail(&mut fives_rx[0], c2),
                cmd.wait_vacant(&mut fizzbuzz_messages_tx, vacant_block)
            );

            let start_value = state.value;
            let remaining = stop_value - state.value;
            let batch_size = BATCH_SIZE.min(remaining as usize);

            // Step 1: Fill the buffer with consecutive numbers
            for i in 0..batch_size {
                state.buffer[i] = FizzBuzzMessage::Value(start_value + i as u64);
            }

            // Step 2: Use iterators from channels to replace values with Fizz, Buzz, or FizzBuzz
            {
                let mut iter_threes = cmd.take_into_iter(&mut threes_rx[0]);

                if state.i_three.is_none() {
                    state.i_three = iter_threes.next();
                }

                while let Some(t) = state.i_three {
                    if t.value < start_value + batch_size as u64 {
                        state.buffer[(t.value - start_value) as usize] = FizzBuzzMessage::Fizz;
                        state.i_three = iter_threes.next();
                    } else {
                        break;
                    }
                }
            }

            {
                let mut iter_fives = cmd.take_into_iter(&mut fives_rx[0]);

                if state.i_five.is_none() {
                    state.i_five = iter_fives.next();
                }

                while let Some(f) = state.i_five {
                    if f.value < start_value + batch_size as u64 {
                        let index = (f.value - start_value) as usize;
                        state.buffer[index] = match state.buffer[index] {
                            FizzBuzzMessage::Fizz => FizzBuzzMessage::FizzBuzz,
                            _ => FizzBuzzMessage::Buzz,
                        };
                        state.i_five = iter_fives.next();
                    } else {
                        break;
                    }
                }
            }

            let buffer_count = batch_size;
            state.value += buffer_count as u64;

            // Step 3: Send the buffer slice
            let _done = cmd.send_slice_until_full(&mut fizzbuzz_messages_tx, &state.buffer[0..buffer_count]);

            if state.value == stop_value {
                cmd.request_graph_stop();
            }

            panic_countdown -= buffer_count as isize;
            if panic_countdown <= 0 {
                panic!("planned periodic panic");
            }
        }

    Ok(())
}



#[cfg(test)]
pub(crate) mod tests {
    use steady_state::*;
    use super::*;
    use crate::*;

    #[test]
    fn test_fizz_buzz_processor() {
       let mut graph = GraphBuilder::for_testing().with_telemetry_metric_features(false).build(());

       let (test_numbers_tx,numbers_rx) = graph.channel_builder().with_capacity(1000).build_channel_bundle::<_,2>();
       let (fizzbuzz_messages_tx,test_fizzbuzz_messages_rx) = graph.channel_builder().with_capacity(1000).build_channel();
       let (errors_tx,_test_errors_tx) = graph.channel_builder().with_capacity(4).build_channel();

       let value = new_state();
        graph.actor_builder()
                   .with_name("UnitTest")
                   .build_spawn( move |context|
                           internal_behavior(context, 15, numbers_rx.clone(), fizzbuzz_messages_tx.clone(), errors_tx.clone(), value.clone())
                    );

        graph.start(); //startup the graph

        test_numbers_tx[0].testing_send_all(vec![NumberMessage{value: 3}
                                                     ,NumberMessage{value: 6}
                                                     ,NumberMessage{value: 9}
                                                     ,NumberMessage{value: 12}
                                                     ,NumberMessage{value: 15}]
                                        ,true);

        test_numbers_tx[1].testing_send_all(vec![NumberMessage{value: 5}
                                                     ,NumberMessage{value: 10}
                                                     ,NumberMessage{value: 15}]
                                        ,true);

        graph.request_stop(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(15));
        let expected = 14;
        assert_steady_rx_eq_count!(&test_fizzbuzz_messages_rx,expected);
        let expected1 = vec!(
            FizzBuzzMessage::Value(1),
            FizzBuzzMessage::Value(2),
            FizzBuzzMessage::Fizz,
            FizzBuzzMessage::Value(4),
            FizzBuzzMessage::Buzz,
        );
        assert_steady_rx_eq_take!(&test_fizzbuzz_messages_rx,expected1);
    }

}