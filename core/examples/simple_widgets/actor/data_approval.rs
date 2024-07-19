use std::error::Error;
use std::time::Duration;

use crate::actor::data_generator::WidgetInventory;
#[warn(unused_imports)]
use log::*;

use steady_state::*;
use steady_state::monitor::LocalMonitor;
use steady_state::{Rx, SteadyRx};
use steady_state::{SteadyTx, Tx};
use crate::actor::data_feedback::FailureFeedback;

const BATCH_SIZE: usize = 2000;

#[derive(Clone, Debug, PartialEq, Copy, Ord, PartialOrd, Eq)]
pub struct ApprovedWidgets {
    pub original_count: u64,
    pub approved_count: u64,
}


pub async fn run(context: SteadyContext
                 , rx: SteadyRx<WidgetInventory>
                 , tx: SteadyTx<ApprovedWidgets>
                 , feedback: SteadyTx<FailureFeedback>
                ) -> Result<(),Box<dyn Error>> {

    let mut monitor = into_monitor!(context,[rx],[tx,feedback]);

    let mut tx = tx.lock().await;
    let mut rx = rx.lock().await;
    let mut feedback = feedback.lock().await;

    let mut buffer = [WidgetInventory { count: 0, _payload: 0, }; BATCH_SIZE];

    while monitor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed() && feedback.mark_closed() ) {

        let _clean = wait_for_all_or_proceed_upon!( monitor.wait_periodic(Duration::from_millis(300))
                            , monitor.wait_avail_units(&mut rx, BATCH_SIZE)
                            ,monitor.wait_vacant_units(&mut tx, BATCH_SIZE)
                            ,monitor.wait_vacant_units(&mut feedback, 1)
        ).await;


        //error!("avail units {} ", monitor.avail_units(&mut rx));

        iterate_once(&mut monitor
                        , &mut rx
                        , &mut tx
                        , &mut feedback
                        , &mut buffer
        );

        monitor.relay_stats_smartly();
    }
    Ok(())
}


// important function break out to ensure we have a point to test on
fn iterate_once<const R: usize, const T: usize>(monitor: &mut LocalMonitor<R, T>
                                              , rx: & mut Rx<WidgetInventory>
                                              , tx: & mut Tx<ApprovedWidgets>
                                              , feedback: & mut Tx<FailureFeedback>
                                              , buf: &mut [WidgetInventory; BATCH_SIZE]) {

        let count = monitor.take_slice(rx, buf);
        let mut approvals: Vec<ApprovedWidgets> = Vec::with_capacity(count);
        for b in buf.iter().take(count) {
            approvals.push(ApprovedWidgets {
                original_count: b.count,
                approved_count: b.count / 2,
            });
            if b.count % 20000 == 0 {

                let _ = monitor.try_send(feedback, FailureFeedback {
                    count: b.count,
                    message: "count is a multiple of 20000".to_string(),
                });
            }
        }

        let sent = monitor.send_slice_until_full(tx, &approvals);
        //iterator of sent until the end
        let send = approvals.into_iter().skip(sent);
        for send_me in send {
            let _ = monitor.try_send(tx, send_me);
        }

}


#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;
    use steady_state::{Graph, util};


    #[test]
    async fn test_process() {
        util::logger::initialize();

        let block_fail_fast = false;
        let mut graph = Graph::internal_new("", block_fail_fast, false);
        let (tx_in, rx_in) = graph.channel_builder().with_capacity(8).build();
        let (tx_out, rx_out) = graph.channel_builder().with_capacity(8).build();
        let (tx_feedback, _rx_feedback) = graph.channel_builder().with_capacity(8).build();
        let tx_in = tx_in.clone();
        let rx_in = rx_in.clone();
        let tx_out = tx_out.clone();
        let rx_out = rx_out.clone();
        let tx_feedback = tx_feedback.clone();


        let mock_monitor = graph.new_test_monitor("approval_monitor");
        let mut mock_monitor = mock_monitor.into_monitor([], []);

        let mut tx_in = tx_in.lock().await;
        let mut rx_in = rx_in.lock().await;

        let mut tx_out = tx_out.lock().await;
        let mut rx_out = rx_out.lock().await;
        let mut tx_feedback = tx_feedback.lock().await;


        let _ = mock_monitor.send_async(&mut tx_in, WidgetInventory {
            count: 5
            , _payload: 42
        },SendSaturation::Warn).await;
        let mut buffer = [WidgetInventory { count: 0, _payload: 0 }; BATCH_SIZE];

        iterate_once(&mut mock_monitor, &mut rx_in, &mut tx_out, &mut tx_feedback, &mut buffer );

        let result = mock_monitor.take_async(&mut rx_out).await.unwrap();
        assert_eq!(result.original_count, 5);
        assert_eq!(result.approved_count, 2);
    }

}