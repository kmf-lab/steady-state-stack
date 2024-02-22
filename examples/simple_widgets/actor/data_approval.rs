use std::ops::DerefMut;
use crate::actor::data_generator::WidgetInventory;
use log::*;
use steady_state::{LocalMonitor, Rx, SteadyContext, SteadyRx, SteadyTx, Tx};
use crate::actor::data_feedback::FailureFeedback;

const BATCH_SIZE: usize = 2000;

#[derive(Clone, Debug, PartialEq, Copy)]
pub struct ApprovedWidgets {
    pub original_count: u64,
    pub approved_count: u64,
}

#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<WidgetInventory>
                 , tx: SteadyTx<ApprovedWidgets>
                 , feedback: SteadyTx<FailureFeedback>
                ) -> Result<(),()> {

    let mut monitor = context.into_monitor([&rx], [&tx,&feedback]);

    let mut tx_guard = tx.lock().await;
    let mut rx_guard = rx.lock().await;
    let mut feedback_guard = feedback.lock().await;

    let tx = tx_guard.deref_mut();
    let rx = rx_guard.deref_mut();
    let feedback = feedback_guard.deref_mut();

    let mut buffer = [WidgetInventory { count: 0, _payload: 0, }; BATCH_SIZE];

    loop {
        //in this example iterate once blocks/await until it has work to do
        //this example is a very responsive telemetry for medium load levels
        //single pass of work, do not loop in here
        if iterate_once(&mut monitor
                        , rx
                        , tx
                        , feedback
                        , &mut buffer
        ).await {
            break Ok(());
        }
        //we relay all our telemetry and return to the top to block for more work.
        monitor.relay_stats_all().await;
    }
}


#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<WidgetInventory>
                 , tx: SteadyTx<ApprovedWidgets>
                 , feedback: SteadyTx<FailureFeedback>
) -> Result<(),()> {

      let mut monitor = context.into_monitor([&rx], [&tx,&feedback]);

      let mut rx_guard = rx.lock().await;
      let mut tx_guard = tx.lock().await;
      let mut feedback_guard = feedback.lock().await;

      let rx = rx_guard.deref_mut();
      let tx = tx_guard.deref_mut();
      let feedback = feedback_guard.deref_mut();

      let mut buffer = [WidgetInventory { count: 0, _payload: 0 }; BATCH_SIZE];

    loop {

        if iterate_once( &mut monitor
                         , rx
                         , tx
                         , feedback
                         , &mut buffer
                         ).await {
            break;
        }


            monitor.relay_stats_all().await;
    }
    Ok(())
}

// important function break out to ensure we have a point to test on
async fn iterate_once<const R: usize, const T: usize>(monitor: &mut LocalMonitor<R, T>
                                                      , rx: & mut Rx<WidgetInventory>
                                                      , tx: & mut Tx<ApprovedWidgets>
                                                      , feedback: & mut Tx<FailureFeedback>
                                                      , buf: &mut [WidgetInventory; BATCH_SIZE]) -> bool {


    if !rx.is_empty() {
        let count = monitor.take_slice(rx, buf);
        let mut approvals: Vec<ApprovedWidgets> = Vec::with_capacity(count);
        for b in buf.iter().take(count) {
            approvals.push(ApprovedWidgets {
                original_count: b.count,
                approved_count: b.count / 2,
            });

            if b.count % 20000 == 0 {
                let _ = monitor.send_async(feedback, FailureFeedback {
                    count: b.count,
                    message: "count is a multiple of 20000".to_string(),
                }).await;
            }
        }

        let sent = monitor.send_slice_until_full(tx, &approvals);
        //iterator of sent until the end
        let send = approvals.into_iter().skip(sent);
        for send_me in send {
            let _ = monitor.send_async(tx, send_me).await;
        }
    } else {

        //by design we wait here for new work
        match monitor.take_async(rx).await {
            Ok(m) => {
                let _ = monitor.send_async(tx, ApprovedWidgets {
                    original_count: m.count,
                    approved_count: m.count / 2,

                }).await;
            },
            Err(msg) => {
                error!("Unexpected error recv_async: {}",msg);
            }
        }
    }

    false
}


#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;
    use steady_state::{Graph, util};


    #[test]
    async fn test_process() {
        util::logger::initialize();

        let mut graph = Graph::new("");
        let (tx_in, rx_in) = graph.channel_builder().with_capacity(8).build();
        let (tx_out, rx_out) = graph.channel_builder().with_capacity(8).build();
        let (tx_feedback, _rx_feedback) = graph.channel_builder().with_capacity(8).build();

        let mock_monitor = graph.new_test_monitor("approval_monitor");
        let mut mock_monitor = mock_monitor.into_monitor([], []);

        let mut tx_in_guard = tx_in.lock().await;
        let mut rx_in_guard = rx_in.lock().await;

        let mut tx_out_guard = tx_out.lock().await;
        let mut rx_out_guard = rx_out.lock().await;
        let mut tx_feedback_guard = tx_feedback.lock().await;

        let tx_in = tx_in_guard.deref_mut();
        let rx_in = rx_in_guard.deref_mut();
        let tx_out = tx_out_guard.deref_mut();
        let rx_out = rx_out_guard.deref_mut();
        let tx_feedback = tx_feedback_guard.deref_mut();

        let _ = mock_monitor.send_async(tx_in, WidgetInventory {
            count: 5
            , _payload: 42
        }).await;
        let mut buffer = [WidgetInventory { count: 0, _payload: 0 }; BATCH_SIZE];

        let exit= iterate_once(&mut mock_monitor, rx_in, tx_out, tx_feedback, &mut buffer ).await;
        assert_eq!(exit, false);

        let result = mock_monitor.take_async(rx_out).await.unwrap();
        assert_eq!(result.original_count, 5);
        assert_eq!(result.approved_count, 2);
    }

}