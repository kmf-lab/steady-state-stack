use std::error::Error;

use crate::actor::data_generator::WidgetInventory;
#[warn(unused_imports)]
use log::*;

use steady_state::*;
use steady_state::monitor::LocalMonitor;
use crate::actor::data_feedback::FailureFeedback;

const BATCH_SIZE: usize = 2000;

#[derive(Clone, Debug, PartialEq, Copy, Ord, PartialOrd, Eq)]
pub struct ApprovedWidgets {
    pub original_count: u64,
    pub approved_count: u64,
}

#[cfg(not(test))]
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

    while monitor.is_running(&mut || rx.is_empty() && rx.is_closed() && tx.mark_closed() && feedback.mark_closed() ) {

        wait_for_all!( monitor.wait_avail_units(&mut rx, 1)
                  , monitor.wait_vacant_units(&mut tx, 1)
                  , monitor.wait_vacant_units(&mut feedback, 1)
        ).await;

        iterate_once(&mut monitor
                        , &mut rx
                        , &mut tx
                        , &mut feedback
                        , &mut buffer
        );

        monitor.relay_stats_smartly().await;
    }
    Ok(())
}


#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<WidgetInventory>
                 , tx: SteadyTx<ApprovedWidgets>
                 , feedback: SteadyTx<FailureFeedback>
) -> Result<(),Box<dyn Error>> {

      let mut monitor = context.into_monitor([&rx], [&tx,&feedback]);

    let mut tx = tx.lock().await;
    let mut rx = rx.lock().await;
    let mut feedback = feedback.lock().await;

      let mut buffer = [WidgetInventory { count: 0, _payload: 0 }; BATCH_SIZE];

    //short circuit logic only closes outgoing if the incoming is empty and closed
    while monitor.is_running(&mut || rx.is_empty() && rx.is_closed() && tx.mark_closed() && feedback.mark_closed()
    ) {

        monitor.wait_avail_units(&mut rx, 1).await;
        monitor.wait_vacant_units(&mut tx, 1).await;
        monitor.wait_vacant_units(&mut feedback, 1).await;

       iterate_once( &mut monitor
                         , &mut rx
                         , &mut tx
                         , &mut feedback
                         , &mut buffer
                         );


        monitor.relay_stats_smartly().await;
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

        let mut graph = Graph::new("");
        let (tx_in, rx_in) = graph.channel_builder().with_capacity(8).build();
        let (tx_out, rx_out) = graph.channel_builder().with_capacity(8).build();
        let (tx_feedback, _rx_feedback) = graph.channel_builder().with_capacity(8).build();

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