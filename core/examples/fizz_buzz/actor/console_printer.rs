
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use std::error::Error;
use std::ops::DerefMut;
use crate::actor::fizz_buzz_processor::FizzBuzzMessage;
use crate::actor::timer_actor::PrintSignal;

pub async fn run(actor: SteadyActorShadow
                 , fizzbuzz_rx: SteadyRx<FizzBuzzMessage>
                 , print_signal_rx: SteadyRx<PrintSignal>) -> Result<(),Box<dyn Error>> {

    let actor = actor.into_spotlight([&fizzbuzz_rx, &print_signal_rx], [] );
    if cfg!(not(test)) {
        internal_behavior(actor, fizzbuzz_rx, print_signal_rx).await
    } else {
        actor.simulated_behavior(vec!(&fizzbuzz_rx, &print_signal_rx)).await
    }
}


const BATCH_SIZE: usize = 20000;
async fn internal_behavior<A: SteadyActor>(mut actor: A
                                           , fizzbuzz_messages_rx: SteadyRx<FizzBuzzMessage>
                                           , print_signal_rx: SteadyRx<PrintSignal>) -> Result<(),Box<dyn Error>> {

    let mut fizzbuzz_messages_rx = fizzbuzz_messages_rx.lock().await;
    let mut print_signal_rx = print_signal_rx.lock().await;

    //boxed so we can allocate more since it is out on the heap.
    let mut buffer:Box<[FizzBuzzMessage; BATCH_SIZE]> = Box::new([FizzBuzzMessage::default(); BATCH_SIZE]);
    let mut total_count:usize = 0;
    let mut total_values:u64 = 0;
    let mut total_fizz:u64 = 0;
    let mut total_buzz:u64 = 0;
    let mut total_fizzbuzz:u64 = 0;

    let wait_for_count = fizzbuzz_messages_rx.capacity()/200;
    while actor.is_running(&mut || i!(fizzbuzz_messages_rx.is_closed_and_empty()) &&
                                               i!(print_signal_rx.is_closed_and_empty())) {

        let _clean = await_for_any!(actor.wait_avail(&mut fizzbuzz_messages_rx, wait_for_count),
                                               actor.wait_avail(&mut print_signal_rx,1));

        while actor.avail_units(&mut fizzbuzz_messages_rx) > 0 {

            let count = actor.take_slice(&mut fizzbuzz_messages_rx, buffer.deref_mut()).item_count();
            buffer[..count].iter().for_each(|msg| match msg {
                FizzBuzzMessage::Fizz => total_fizz += 1,
                FizzBuzzMessage::Buzz => total_buzz += 1,
                FizzBuzzMessage::FizzBuzz => total_fizzbuzz += 1,
                FizzBuzzMessage::Value(_) => total_values += 1,
            });
            total_count += count;
        }

        if let Some(_t) = actor.try_take(&mut print_signal_rx) {
            println!("Total:{} Fizz:{}({}%) Buzz:{}({}%) FizzBuzz:{}({}%) values:{}", total_count,
                     total_fizz, ((total_fizzbuzz + total_fizz) as f64 * 100f64) / total_count as f64,
                     total_buzz, ((total_fizzbuzz + total_buzz) as f64 * 100f64) / total_count as f64,
                     total_fizzbuzz, (total_fizzbuzz as f64 * 100f64) / total_count as f64,
                     total_values);
        };

    }
    println!("Final Total:{} Fizz:{}({}%) Buzz:{}({}%) FizzBuzz:{}({}%) values:{}", total_count,
             total_fizz,((total_fizzbuzz+total_fizz) as f64 * 100f64)/total_count as f64,
             total_buzz,((total_fizzbuzz+total_buzz) as f64 * 100f64)/total_count as f64,
             total_fizzbuzz,(total_fizzbuzz as f64 * 100f64)/total_count as f64,
             total_values);
    Ok(())
}



#[cfg(test)]
pub(crate) mod tests {
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::*;
    use super::*;

    #[test]
    fn test_console_printer() -> Result<(),Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(());
        let (test_fizzbuzz_messages_tx,fizzbuzz_messages_rx) = graph.channel_builder().with_capacity(40).build_channel();
        let (test_print_signal_tx,print_signal_rx) = graph.channel_builder().with_capacity(4).build_channel();

        graph.actor_builder()
            .with_name("UnitTest")
            .build( move |context|
                internal_behavior(context,fizzbuzz_messages_rx.clone(),print_signal_rx.clone()), SoloAct
            );

        graph.start(); //startup the graph
        test_fizzbuzz_messages_tx.testing_send_all(vec![
            FizzBuzzMessage::Value(1),
            FizzBuzzMessage::Value(2),
            FizzBuzzMessage::Fizz,
            FizzBuzzMessage::Value(4),
            FizzBuzzMessage::Buzz,
            FizzBuzzMessage::Fizz,
            FizzBuzzMessage::Value(7),
            FizzBuzzMessage::Value(8),
            FizzBuzzMessage::Fizz,
            FizzBuzzMessage::Buzz,
            FizzBuzzMessage::Value(11),
            FizzBuzzMessage::Fizz,
            FizzBuzzMessage::Value(13),
            FizzBuzzMessage::Value(14),
            FizzBuzzMessage::FizzBuzz,
        ],true);
        sleep(Duration::from_millis(2));

        test_print_signal_tx.testing_send_all(vec![PrintSignal { tick: 1 },
                                                       ],true);
        sleep(Duration::from_millis(1));

        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(4))

        //nothing to test as this will print to the console
    }


}