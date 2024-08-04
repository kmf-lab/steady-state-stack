use std::error::Error;
use std::time::Duration;

use crate::actor::data_generator::WidgetInventory;
#[warn(unused_imports)]
use log::*;

use steady_state::*;

use steady_state::{SteadyRx};
use steady_state::{SteadyTx};
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
    internal_behavior(context, rx, tx, feedback).await
}

async fn internal_behavior(context: SteadyContext, rx: SteadyRx<WidgetInventory>, tx: SteadyTx<ApprovedWidgets>, feedback: SteadyTx<FailureFeedback>) -> Result<(), Box<dyn Error>> {
    let mut monitor = into_monitor!(context,[rx],[tx,feedback]);

    let mut tx = tx.lock().await;
    let mut rx = rx.lock().await;
    let mut feedback = feedback.lock().await;

    let mut buffer = [WidgetInventory { count: 0, _payload: 0, }; BATCH_SIZE];

    while monitor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed() && feedback.mark_closed()) {

        if monitor.is_liveliness_in(&[GraphLivelinessState::StopRequested],false) {
            info!("Stop requested, exiting");
        }

        let _clean = wait_for_all_or_proceed_upon!(monitor.wait_periodic(Duration::from_millis(300))
                            ,monitor.wait_avail_units(&mut rx, BATCH_SIZE)
                            ,monitor.wait_vacant_units(&mut tx, BATCH_SIZE)
                            ,monitor.wait_vacant_units(&mut feedback, 1)
        ).await;

        let count = monitor.take_slice(&mut rx, &mut buffer);
        let mut approvals: Vec<ApprovedWidgets> = Vec::with_capacity(count);
        for b in buffer.iter().take(count) {
            approvals.push(ApprovedWidgets {
                original_count: b.count,
                approved_count: b.count / 2,
            });
            if b.count % 20000 == 0 {

                let _ = monitor.try_send(&mut feedback, FailureFeedback {
                    count: b.count,
                    message: "count is a multiple of 20000".to_string(),
                });
            }
        }

        let sent = monitor.send_slice_until_full(&mut tx, &approvals);
        //iterator of sent until the end
        let send = approvals.into_iter().skip(sent);
        for send_me in send {
            let _ = monitor.try_send(&mut tx, send_me);
        }

        monitor.relay_stats_smartly();
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod actor_tests {
    use std::time::Duration;
    use async_std::test;
    use steady_state::*;
    use crate::actor::data_approval::{BATCH_SIZE, internal_behavior};
    use crate::actor::WidgetInventory;

    #[test]
    pub(crate) async fn test_simple_process() {
        //1. build test graph, the input and output channels and our actor
        let mut graph = Graph::new_test(());

        let (widget_inventory_tx_in, widget_inventory_rx_in) = graph.channel_builder()
            .with_capacity(BATCH_SIZE).build();
        let (approved_widget_tx_out,approved_widget_rx_out) = graph.channel_builder()
            .with_capacity(BATCH_SIZE).build();
        let (feedback_tx_out,feedback_rx_out) = graph.channel_builder()
            .with_capacity(BATCH_SIZE).build();
        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context| internal_behavior(context
                                                           , widget_inventory_rx_in.clone()
                                                           , approved_widget_tx_out.clone()
                                                           , feedback_tx_out.clone()) );

        // //2. add test data to the input channels
        let test_data:Vec<WidgetInventory> = (0..BATCH_SIZE).map(|i| WidgetInventory { count: i as u64, _payload: 0 }).collect();
        widget_inventory_tx_in.testing_send(test_data, Duration::from_millis(30),true).await;

        //3. run graph until the actor detects the input is closed
        graph.start_as_data_driven(Duration::from_secs(240));

        //4. assert expected results
        assert_eq!(approved_widget_rx_out.testing_avail_units().await, BATCH_SIZE);
    }
}