use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::SteadyRx;
use steady_state::SteadyTx;

#[derive(Clone, Debug, PartialEq)]
pub struct FailureFeedback {
    pub count: u64,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq)]
    pub struct ChangeRequest {
    pub msg: FailureFeedback,
}


pub async fn run(context: SteadyContext
                 , rx: SteadyRx<FailureFeedback>
                 , tx: SteadyTx<ChangeRequest>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context.into_monitor([&rx], [&tx]), rx, tx).await
}

async fn internal_behavior<C: SteadyCommander>(mut cmd:C, rx: SteadyRx<FailureFeedback>, tx: SteadyTx<ChangeRequest>) -> Result<(), Box<dyn Error>> {

    let mut tx = tx.lock().await;
    let mut rx = rx.lock().await;


    while cmd.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()) {

        let _clean = await_for_all!(   cmd.wait_avail(&mut rx,1)
                                     ,cmd.wait_vacant(&mut tx,1)   );

        //in this example iterate once blocks/await until it has work to do
        //this example is a very responsive telemetry for medium load levels
        //single pass of work, do not loop in here
        if let Some(msg) = cmd.take_async(&mut rx).await {
            //we have a message to process
            //we do not care about the message we just need to send a change request
            let _ = cmd.try_send(&mut tx, ChangeRequest {msg});
        }

        //we relay all our telemetry and return to the top to block for more work.
        cmd.relay_stats_smartly();
    }
    Ok(())
}



#[cfg(test)]
mod tests {
    use std::thread::sleep;
    use std::time::Duration;
    use futures_timer::Delay;
    use steady_state::*;
    use crate::actor::data_feedback::internal_behavior;

    #[test]
    fn test_feedback() {
        
        const BATCH_SIZE:usize = 200;
        
        let mut graph = GraphBuilder::for_testing().build(());

        let (failure_feedback_tx_out,failure_feedback_rx_out) = graph.channel_builder()
                                                                   .with_capacity(20)
                                                                   .build();
        
        let (change_request_tx_out,change_request_rx_out) = graph.channel_builder()
                                                                    .with_capacity(BATCH_SIZE)
                                                                    .build();
        
         graph.actor_builder()
             .with_name("UnitTest")
             .build_spawn(move |context| internal_behavior(context, failure_feedback_rx_out.clone(), change_request_tx_out.clone()));

        failure_feedback_tx_out.testing_send_all((0..10).map(|i| crate::actor::data_feedback::FailureFeedback { count: i, message: "test".to_string() }).collect(),true);
        graph.start();
        sleep(Duration::from_millis(60));
        graph.request_stop();
        graph.block_until_stopped(Duration::from_secs(15));
        let expected = 10;
        assert_steady_rx_eq_count!(&change_request_rx_out,expected);
    }
}