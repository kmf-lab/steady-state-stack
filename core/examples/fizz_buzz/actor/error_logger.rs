
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use std::error::Error;
use crate::actor::fizz_buzz_processor::ErrorMessage;


pub async fn run(actor: SteadyActorShadow
                 , errors_rx: SteadyRx<ErrorMessage>) -> Result<(),Box<dyn Error>> {

    let actor = actor.into_spotlight([&errors_rx], []);
    if cfg!(not(test)) {
        internal_behavior(actor, errors_rx).await
    } else {
        actor.simulated_behavior(vec!(&errors_rx)).await
    }
}


async fn internal_behavior<A: SteadyActor>(mut actor: A
                                           , errors_rx: SteadyRx<ErrorMessage>) -> Result<(),Box<dyn Error>> {

    let mut errors_rx = errors_rx.lock().await;

    while actor.is_running(&mut || i!(errors_rx.is_closed_and_empty())) {

         let clean = await_for_all!(actor.wait_avail(&mut errors_rx,1) );

         match actor.try_take(&mut errors_rx) {
                Some(_message) => {
                    #[cfg(not(test))]
                    error!("Error: {:?}",_message);
                    actor.relay_stats();
                },
                None => {
                    if clean {
                        error!("internal error, should have found message");
                    }
                }
         };
    }
    Ok(())
}




#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;
    use steady_state::*;
    use super::*;

    #[test]
    fn test_error_logger() -> Result<(),Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(());
        let (test_errors_tx,errors_rx) = graph.channel_builder().with_capacity(4).build_channel();
        graph.actor_builder()
            .with_name("UnitTest")
            .build( move |context|
                internal_behavior(context,errors_rx.clone()), SoloAct
            );

        graph.start(); //startup the graph

        test_errors_tx.testing_send_all(vec![
                        ErrorMessage { text: "ignore me from testing, error 1".to_string() },
                        ErrorMessage { text: "ignore me from testing, error 2".to_string() },
                        ErrorMessage { text: "ignore me from testing, error 3".to_string() },
                    ], true);
        graph.request_shutdown(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(1))

        //nothing to test logger will just write to console errors we ignore

    }


}