use std::error::Error;
use std::time::Duration;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::{Rx, SteadyRx};
use steady_state::{SteadyTx, Tx};
use crate::actor::data_generator::Packet;

pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTx<Packet>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, rx, tx).await
}

async fn internal_behavior(context: SteadyContext, rx: SteadyRx<Packet>, tx: SteadyTx<Packet>) -> Result<(), Box<dyn Error>> {
    //info!("running {:?} {:?}",context.id(),context.name());

    let mut monitor = into_monitor!(context, [rx], [tx]);

    //guards for the channels, NOTE: we could share one channel across actors.
    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = rx.capacity().min(tx.capacity()) / 2;


    while monitor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()) {

        let _clean = wait_for_all_or_proceed_upon!(
             monitor.wait_periodic(Duration::from_millis(20))
            ,monitor.wait_avail_units(&mut rx,count)
            ,monitor.wait_vacant_units(&mut tx,count)
        );

        let count = monitor.avail_units(&mut rx).min(monitor.vacant_units(&mut tx));
        if count > 0 {
            for _ in 0..count {
                if let Some(packet) = monitor.try_take(&mut rx) {
                    if let Err(e) = monitor.try_send(&mut tx, packet) {
                        error!("Error sending packet: {:?}",e);
                        break;
                    }
                } else {
                    error!("Error reading packet");
                    break;
                }
            }
            monitor.relay_stats_smartly();
        }
    }
    Ok(())
}


#[cfg(test)]
mod process_tests {
    use std::time::Duration;
    use super::*;
    use async_std::test;
    use steady_state::Graph;


    // #[test]
    // pub(crate) async fn test_process() {
    //     //1. build test graph, the input and output channels and our actor
    //     let mut graph = Graph::new_test(());
    //
    //     // let (approved_widget_tx_out, approved_widget_rx_out) = graph.channel_builder()
    //     //     .with_capacity(BATCH_SIZE).build();
    //     //
    //     // let state = InternalState {
    //     //     last_approval: None,
    //     //     buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
    //     // };
    //
    //     // graph.actor_builder()
    //     //     .with_name("UnitTest")
    //     //     .build_spawn(move |context| internal_behavior(context, approved_widget_rx_out.clone(), state));
    //     //
    //     // // //2. add test data to the input channels
    //     // let test_data: Vec<Packet> = (0..BATCH_SIZE).map(|i| Packet { original_count: 0, approved_count: i as u64 }).collect();
    //     // approved_widget_tx_out.testing_send(test_data, Duration::from_millis(30), true).await;
    //
    //     // //3. run graph until the actor detects the input is closed
    //     graph.start_as_data_driven(Duration::from_secs(240));
    //
    //     //4. assert expected results
    //     // TODO: not sure how to make this work.
    //     //  println!("last approval: {:?}", &state.last_approval);
    //     //  assert_eq!(approved_widget_rx_out.testing_avail_units().await, BATCH_SIZE);
    // }


}


