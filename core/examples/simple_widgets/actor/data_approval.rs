use std::error::Error;
use std::time::Duration;

use crate::actor::data_generator::WidgetInventory;
#[warn(unused_imports)]
use log::*;

use steady_state::*;

use steady_state::SteadyRx;
use steady_state::SteadyTx;
use crate::actor::data_feedback::FailureFeedback;

const BATCH_SIZE: usize = 2000;

#[derive(Clone, Debug, PartialEq, Copy, Ord, PartialOrd, Eq)]
pub struct ApprovedWidgets {
    pub original_count: u64,
    pub approved_count: u64,
}


pub async fn run(context: SteadyActorShadow
                 , rx: SteadyRx<WidgetInventory>
                 , tx: SteadyTx<ApprovedWidgets>
                 , feedback: SteadyTx<FailureFeedback>
                ) -> Result<(),Box<dyn Error>> {
    let actor = context.into_spotlight([&rx], [&tx,&feedback]);
    if actor.use_internal_behavior {
        internal_behavior(actor, rx, tx, feedback).await
    } else {
        actor.simulated_behavior(sim_runners!(rx, tx, feedback)).await
    }
}

async fn internal_behavior<C: SteadyActor>(mut actor: C, rx: SteadyRx<WidgetInventory>, tx: SteadyTx<ApprovedWidgets>, feedback: SteadyTx<FailureFeedback>) -> Result<(), Box<dyn Error>> {

    let mut tx = tx.lock().await;
    let mut rx = rx.lock().await;
    let mut feedback = feedback.lock().await;

    let mut buffer = [WidgetInventory { count: 0, _payload: 0, }; BATCH_SIZE];

    while actor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed() && feedback.mark_closed()) {

        let _clean = await_for_all_or_proceed_upon!(actor.wait_periodic(Duration::from_millis(300))
                            ,actor.wait_avail(&mut rx, BATCH_SIZE)
                            ,actor.wait_vacant(&mut tx, BATCH_SIZE)
                            ,actor.wait_vacant(&mut feedback, 1)
        );

        let count = actor.take_slice(&mut rx, &mut buffer).item_count();
        let mut approvals: Vec<ApprovedWidgets> = Vec::with_capacity(count);
        for b in buffer.iter().take(count) {
            approvals.push(ApprovedWidgets {
                original_count: b.count,
                approved_count: b.count / 2,
            });
            if b.count % 20000 == 0 {

                let _ = actor.try_send(&mut feedback, FailureFeedback {
                    count: b.count,
                    message: "count is a multiple of 20000".to_string(),
                });
            }
        }

        let done = actor.send_slice(&mut tx, &approvals);
        //iterator of sent until the end
        let send = approvals.into_iter().skip(done.item_count());
        for send_me in send {
            let _ = actor.try_send(&mut tx, send_me);
        }

        actor.relay_stats_smartly();
    }
    Ok(())
}


#[cfg(test)]
pub(crate) mod approval_tests {
    use std::time::Duration;
    use steady_state::*;
    use crate::actor::data_approval::{internal_behavior, BATCH_SIZE};
    use crate::actor::WidgetInventory;

    #[test]
    fn test_approval() -> Result<(), Box<dyn Error>> {
        // build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing().build(());

        let (widget_inventory_tx_in, widget_inventory_rx_in) = graph.channel_builder()
            .with_capacity(BATCH_SIZE).build_channel();
        let (approved_widget_tx_out,approved_widget_rx_out) = graph.channel_builder()
            .with_capacity(BATCH_SIZE).build_channel();

        let (feedback_tx_out,_feedback_rx_out) = graph.channel_builder()
            .with_capacity(BATCH_SIZE).build_channel();

        graph.actor_builder()
            .with_name("UnitTest")
            .build( move |context| internal_behavior(context
                                                           , widget_inventory_rx_in.clone()
                                                           , approved_widget_tx_out.clone()
                                                           , feedback_tx_out.clone()), SoloAct );

        graph.start();

       //
       let test_data:Vec<WidgetInventory> = (0..BATCH_SIZE).map(|i| WidgetInventory { count: i as u64, _payload: 0 }).collect();
       widget_inventory_tx_in.testing_send_all(test_data, true);
       //
       graph.request_shutdown();
       graph.block_until_stopped(Duration::from_secs(2))?;
       //
       assert_steady_rx_eq_count!(&approved_widget_rx_out,BATCH_SIZE);
        Ok(())
    }




}
