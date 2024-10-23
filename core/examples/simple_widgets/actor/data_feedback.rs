use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::{SteadyRx};
use steady_state::{SteadyTx};

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
    internal_behavior(context, rx, tx).await
}

async fn internal_behavior(context: SteadyContext, rx: SteadyRx<FailureFeedback>, tx: SteadyTx<ChangeRequest>) -> Result<(), Box<dyn Error>> {
    //trace!("running {:?} {:?}",context.id(),context.name());

    let mut monitor = into_monitor!(context, [rx], [tx]);

    let mut tx = tx.lock().await;
    let mut rx = rx.lock().await;


    while monitor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()) {

        let _clean = wait_for_all!(   monitor.wait_shutdown_or_avail_units(&mut rx,1)
                                     ,monitor.wait_shutdown_or_vacant_units(&mut tx,1)   );

        //in this example iterate once blocks/await until it has work to do
        //this example is a very responsive telemetry for medium load levels
        //single pass of work, do not loop in here
        if let Some(msg) = monitor.take_async(&mut rx).await {
            //we have a message to process
            //we do not care about the message we just need to send a change request
            let _ = monitor.try_send(&mut tx,ChangeRequest {msg});
        }

        //we relay all our telemetry and return to the top to block for more work.
        monitor.relay_stats_smartly();
    }
    Ok(())
}



#[cfg(test)]
mod tests {
    use steady_state::GraphBuilder;

    #[async_std::test]
    async fn test_feedback() {
        let graph = GraphBuilder::for_testing().build(());
        
    //     let graph = Graph::new_test(());        let (approved_widget_tx_out,approved_widget_rx_out) = graph.channel_builder()
    //         .with_capacity(BATCH_SIZE).build();
    //
    //     let state = InternalState {
    //         last_approval: None,
    //         buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
    //     };
    //
    //     graph.actor_builder()
    //         .with_name("UnitTest")
    //         .build_spawn(move |context| internal_behavior(context, approved_widget_rx_out.clone(), state));
    //
    //     // //2. add test data to the input channels
    //     let test_data: Vec<ApprovedWidgets> = (0..BATCH_SIZE).map(|i| ApprovedWidgets { original_count: 0, approved_count: i as u64 }).collect();
    //     approved_widget_tx_out.testing_send(test_data, Duration::from_millis(30), true).await;
    //
    //     // //3. run graph until the actor detects the input is closed
    //     graph.start_as_data_driven(Duration::from_secs(240));
    //
    //     //4. assert expected results
    //     // TODO: not sure how to make this work.
    //     //  println!("last approval: {:?}", &state.last_approval);
    //     //  assert_eq!(approved_widget_rx_out.testing_avail_units().await, BATCH_SIZE);
    //
    
    
    }



}