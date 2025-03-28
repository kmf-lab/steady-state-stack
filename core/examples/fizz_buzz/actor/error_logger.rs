
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use std::error::Error;
use crate::actor::fizz_buzz_processor::ErrorMessage;


pub async fn run(context: SteadyContext
        ,errors_rx: SteadyRx<ErrorMessage>) -> Result<(),Box<dyn Error>> {

    let cmd = context.into_monitor([&errors_rx],[]);
    if cfg!(not(test)) {
        internal_behavior(cmd, errors_rx).await
    } else {
        cmd .simulated_behavior(vec!(&TestEquals(errors_rx))).await
    }
}


async fn internal_behavior<C:SteadyCommander>(mut cmd: C
                ,errors_rx: SteadyRx<ErrorMessage>) -> Result<(),Box<dyn Error>> {

    let mut errors_rx = errors_rx.lock().await;

    while cmd.is_running(&mut || errors_rx.is_closed_and_empty()) {

         let clean = await_for_all!(cmd.wait_avail(&mut errors_rx,1) );

         match cmd.try_take(&mut errors_rx) {
                Some(message) => {
                    error!("Error: {:?}",message);
                    cmd.relay_stats();
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
    fn test_simple_process() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (test_errors_tx,errors_rx) = graph.channel_builder().with_capacity(4).build();
        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context|
                internal_behavior(context,errors_rx.clone())
            );

        graph.start(); //startup the graph

        test_errors_tx.testing_send_all(vec![
                        ErrorMessage { text: "ignore me from testing, error 1".to_string() },
                        ErrorMessage { text: "ignore me from testing, error 2".to_string() },
                        ErrorMessage { text: "ignore me from testing, error 3".to_string() },
                    ], true);
        graph.request_stop(); //our actor has no input so it immediately stops upon this request
        assert!(graph.block_until_stopped(Duration::from_secs(1)));

        //nothing to test logger will just write to console errors we ignore

    }


}