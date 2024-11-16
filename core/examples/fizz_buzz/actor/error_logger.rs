
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use crate::Args;


use std::error::Error;
use steady_state::commander::SteadyCommander;
use crate::actor::fizz_buzz_processor::ErrorMessage;






//if no internal state is required (recommended) feel free to remove this.
#[derive(Default)]
struct ErrorloggerInternalState {
     
     
}
impl ErrorloggerInternalState {
    fn new(cli_args: &Args) -> Self {
        Self {
           ////TODO: : add custom arg based init here
           ..Default::default()
        }
    }
}

#[cfg(not(test))]
pub async fn run(context: SteadyContext
        ,errors_rx: SteadyRx<ErrorMessage>) -> Result<(),Box<dyn Error>> {
  internal_behavior(context,errors_rx).await
}

async fn internal_behavior(context: SteadyContext
        ,errors_rx: SteadyRx<ErrorMessage>) -> Result<(),Box<dyn Error>> {

    // here is how to access the CLI args if needed
    let cli_args = context.args::<Args>();

    // here is how to initialize the internal state if needed
    let mut state = if let Some(args) = cli_args {
        ErrorloggerInternalState::new(args)
    } else {
        ErrorloggerInternalState::default()
    };

    // monitor consumes context and ensures all the traffic on the passed channels is monitored
    let mut monitor =  into_monitor!(context, [
                        errors_rx],[]
                           );

   //every channel must be locked before use, if this actor should panic the lock will be released
   //and the replacement actor will lock them here again
 
    let mut errors_rx = errors_rx.lock().await;
 

    //this is the main loop of the actor, will run until shutdown is requested.
    //the closure is called upon shutdown to determine if we need to postpone the shutdown for this actor
    while monitor.is_running(&mut ||
    errors_rx.is_closed_and_empty()) {

         let _clean = await_for_all!(monitor.wait_shutdown_or_avail_units(&mut errors_rx,1)    );


     //TODO:  here are all the channels you can read from
          let errors_rx_ref: &mut Rx<ErrorMessage> = &mut errors_rx;

     //TODO:  here are all the channels you can write to

     //TODO:  to get started try calling the monitor.* methods:
      //    try_take<T>(&mut self, this: &mut Rx<T>) -> Option<T>  ie monitor.try_take(...
      //    try_send<T>(&mut self, this: &mut Tx<T>, msg: T) -> Result<(), T>  ie monitor.try_send(...

     monitor.relay_stats_smartly();

    }
    Ok(())
}


#[cfg(test)]
pub async fn run(context: SteadyContext
        ,errors_rx: SteadyRx<ErrorMessage>) -> Result<(),Box<dyn Error>> {


    let mut monitor =  into_monitor!(context, [
                            errors_rx],[]
                               );

    if let Some(responder) = monitor.sidechannel_responder() {

         
            let mut errors_rx = errors_rx.lock().await;
         

         while monitor.is_running(&mut ||
             errors_rx.is_closed_and_empty()) {

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
       //let (errors_rx,test_errors_tx) = graph.channel_builder().with_capacity(4).build()
       //TODO:  you may need to use .build() or  .build_as_bundle::<_, SOME_VALUE>()//TODO:  uncomment to add your test
        //graph.actor_builder()
        //            .with_name("UnitTest")
        //            .build_spawn( move |context|
        //                    internal_behavior(context,errors_rx.clone())
        //             );

        graph.start(); //startup the graph

        //TODO:  add your test values here

        graph.request_stop(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(15));

        //TODO:  confirm values on the output channels
        //    assert_eq!(XX_rx_out[0].testing_avail_units().await, 1);
    }


}