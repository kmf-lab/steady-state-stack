use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::monitor::LocalMonitor;
use steady_state::{Rx, SteadyRx};
use steady_state::{SteadyTx, Tx};

#[derive(Clone, Debug, PartialEq)]
pub struct FailureFeedback {
    pub count: u64,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq)]
    pub struct ChangeRequest {
    pub msg: FailureFeedback,
}

#[cfg(not(test))]
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
        //in this example iterate once blocks/await until it has work to do
        //this example is a very responsive telemetry for medium load levels
        //single pass of work, do not loop in here
        iterate_once(&mut monitor
                     , &mut rx
                     , &mut tx
        ).await;
        //we relay all our telemetry and return to the top to block for more work.
        monitor.relay_stats_smartly();
    }
    Ok(())
}

#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<FailureFeedback>
                 , tx: SteadyTx<ChangeRequest>) -> Result<(),Box<dyn Error>> {

    let mut monitor = into_monitor!(context, [rx], [tx]);
    let mut tx = tx.lock().await;
    let mut rx = rx.lock().await;

    while monitor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()  ) {

        iterate_once( &mut monitor
                         , &mut rx
                         , &mut tx
                         );


         monitor.relay_stats_smartly();
    }
    Ok(())
}

// important function break out to ensure we have a point to test on
async fn iterate_once<const R: usize, const T: usize>(monitor: &mut LocalMonitor<R, T>
                                                      , rx: & mut Rx<FailureFeedback>
                                                      , tx: & mut Tx<ChangeRequest>
) -> bool {

    if let Some(msg) = monitor.take_async(rx).await {
        //we have a message to process
        //we do not care about the message we just need to send a change request
        let _ = monitor.send_async(tx,ChangeRequest {msg},SendSaturation::Warn).await;
    }

    false
}


#[cfg(test)]
mod tests {
    use async_std::test;
    use steady_state::*;


    #[test]
    async fn test_process() {
        util::logger::initialize();
        let block_fail_fast = false;
        let graph = Graph::internal_new((), block_fail_fast, false);
        let _mock_monitor = graph.new_test_monitor("test_monitor");

    /*

        let (tx_in, rx_in) = graph.channel_builder().with_capacity(8).build();
        let (tx_out, rx_out) = graph.channel_builder().with_capacity(8).build();

        let mut mock_monitor = mock_monitor.into_monitor(&mut[], &mut[]);

        let mut tx_in_guard = tx_in.lock().await;
        let mut rx_in_guard = rx_in.lock().await;
        let mut tx_out_guard = tx_out.lock().await;
        let mut rx_out_guard = rx_out.lock().await;

        let tx_in = tx_in_guard.deref_mut();
        let rx_in = rx_in_guard.deref_mut();
        let tx_out = tx_out_guard.deref_mut();
        let rx_out = rx_out_guard.deref_mut();

        let _ = mock_monitor.send_async(tx_in, WidgetInventory {
            count: 5
            , _payload: 42
        }).await;
        let mut buffer = [WidgetInventory { count: 0, _payload: 0 }; BATCH_SIZE];

        let exit= iterate_once(&mut mock_monitor, rx_in, tx_out, &mut buffer ).await;
        assert_eq!(exit, false);

        let result = mock_monitor.take_async(rx_out).await.unwrap();
        assert_eq!(result.original_count, 5);
        assert_eq!(result.approved_count, 2);
    */
    }



}