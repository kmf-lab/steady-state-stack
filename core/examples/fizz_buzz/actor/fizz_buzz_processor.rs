
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;


use std::error::Error;
use crate::actor::div_by_3_producer::NumberMessage;


#[derive(Default)]
pub(crate) struct ErrorMessage {
   dummy: u8 //TODO: : replace this and put your fields here
}
#[derive(Default)]
pub(crate) struct FizzBuzzMessage {
   dummy: u8 //TODO: : replace this and put your fields here
}




//if no internal state is required (recommended) feel free to remove this.
#[derive(Default)]
struct FizzbuzzprocessorInternalState {
     
     
     
     
}
impl FizzbuzzprocessorInternalState {
    fn new(cli_args: &Args) -> Self {
        Self {
           ////TODO: : add custom arg based init here
           ..Default::default()
        }
    }
}

#[cfg(not(test))]
pub async fn run<const NUMBERS_RX_GIRTH:usize,>(context: SteadyContext
        ,numbers_rx: SteadyRxBundle<NumberMessage, NUMBERS_RX_GIRTH>
        ,fizzbuzz_messages_tx: SteadyTx<FizzBuzzMessage>
        ,errors_tx: SteadyTx<ErrorMessage>) -> Result<(),Box<dyn Error>> {
  internal_behavior(context,numbers_rx,fizzbuzz_messages_tx,errors_tx).await
}

async fn internal_behavior<const NUMBERS_RX_GIRTH:usize,>(context: SteadyContext
        ,numbers_rx: SteadyRxBundle<NumberMessage, NUMBERS_RX_GIRTH>
        ,fizzbuzz_messages_tx: SteadyTx<FizzBuzzMessage>
        ,errors_tx: SteadyTx<ErrorMessage>) -> Result<(),Box<dyn Error>> {

    // here is how to access the CLI args if needed
    let cli_args = context.args::<Args>();

    // here is how to initialize the internal state if needed
    let mut state = if let Some(args) = cli_args {
        FizzbuzzprocessorInternalState::new(args)
    } else {
        FizzbuzzprocessorInternalState::default()
    };

    // monitor consumes context and ensures all the traffic on the passed channels is monitored
    let mut monitor =  into_monitor!(context, [
                        numbers_rx[0],
                        numbers_rx[1]],[
                        fizzbuzz_messages_tx,
                        errors_tx]
                           );

   //every channel must be locked before use, if this actor should panic the lock will be released
   //and the replacement actor will lock them here again
 
    let mut numbers_rx = numbers_rx.lock().await;
 
    let mut fizzbuzz_messages_tx = fizzbuzz_messages_tx.lock().await;
 
    let mut errors_tx = errors_tx.lock().await;
 

    //this is the main loop of the actor, will run until shutdown is requested.
    //the closure is called upon shutdown to determine if we need to postpone the shutdown for this actor
    while monitor.is_running(&mut ||
    numbers_rx.is_closed_and_empty() && fizzbuzz_messages_tx.mark_closed() && errors_tx.mark_closed()) {

         let _clean = await_for_all!(monitor.wait_shutdown_or_avail_units_bundle(&mut numbers_rx,1,1)    );


     //TODO:  here are all the channels you can read from
          let numbers_rx_ref: &mut RxBundle<'_, NumberMessage> = &mut numbers_rx;

     //TODO:  here are all the channels you can write to
          let fizzbuzz_messages_tx_ref: &mut Tx<FizzBuzzMessage> = &mut fizzbuzz_messages_tx;
          let errors_tx_ref: &mut Tx<ErrorMessage> = &mut errors_tx;

     //TODO:  to get started try calling the monitor.* methods:
      //    try_take<T>(&mut self, this: &mut Rx<T>) -> Option<T>  ie monitor.try_take(...
      //    try_send<T>(&mut self, this: &mut Tx<T>, msg: T) -> Result<(), T>  ie monitor.try_send(...

     monitor.relay_stats_smartly();

    }
    Ok(())
}


#[cfg(test)]
pub async fn run<const NUMBERS_RX_GIRTH:usize,>(context: SteadyContext
        ,numbers_rx: SteadyRxBundle<NumberMessage, NUMBERS_RX_GIRTH>
        ,fizzbuzz_messages_tx: SteadyTx<FizzBuzzMessage>
        ,errors_tx: SteadyTx<ErrorMessage>) -> Result<(),Box<dyn Error>> {


    let mut monitor =  into_monitor!(context, [
                            numbers_rx[0],
                            numbers_rx[1]],[
                            fizzbuzz_messages_tx,
                            errors_tx]
                               );

    if let Some(responder) = monitor.sidechannel_responder() {

         
            let mut numbers_rx = numbers_rx.lock().await;
         
            let mut fizzbuzz_messages_tx = fizzbuzz_messages_tx.lock().await;
         
            let mut errors_tx = errors_tx.lock().await;
         

         while monitor.is_running(&mut ||
             numbers_rx.is_closed_and_empty() && fizzbuzz_messages_tx.mark_closed() && errors_tx.mark_closed()) {

                //TODO:  write responder code:: let responder = responder.respond_with(|message| {

                monitor.relay_stats_smartly();
         }

    }

    Ok(())

}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;
    use steady_state::*;
    use super::*;


    #[async_std::test]
    pub(crate) async fn test_simple_process() {
       let mut graph = GraphBuilder::for_testing().build(());

       //TODO:  you may need to use .build() or  .build_as_bundle::<_, SOME_VALUE>()
       //let (numbers_rx,test_numbers_tx) = graph.channel_builder().with_capacity(4).build()
       //TODO:  you may need to use .build() or  .build_as_bundle::<_, SOME_VALUE>()
       //let (test_fizzbuzz_messages_rx,fizzbuzz_messages_tx) = graph.channel_builder().with_capacity(4).build()
       
       //let (test_errors_rx,errors_tx) = graph.channel_builder().with_capacity(4).build()
       //TODO:  uncomment to add your test
        //graph.actor_builder()
        //            .with_name("UnitTest")
        //            .build_spawn( move |context|
        //                    internal_behavior(context,numbers_rx.clone(),fizzbuzz_messages_tx.clone(),errors_tx.clone())
        //             );

        graph.start(); //startup the graph

        //TODO:  add your test values here

        graph.request_stop(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(15));

        //TODO:  confirm values on the output channels
        //    assert_eq!(XX_rx_out[0].testing_avail_units().await, 1);
    }


}