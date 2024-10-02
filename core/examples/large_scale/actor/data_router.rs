use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use crate::actor::data_generator::Packet;

//use futures::future::FutureExt;
use std::time::Duration;

use steady_state::{SteadyRx};
use steady_state::{SteadyTxBundle};

pub async fn run<const GIRTH:usize>(context: SteadyContext
                 , one_of: usize
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTxBundle<Packet,GIRTH>
                ) -> Result<(),Box<dyn Error>> {

    internal_behavior(context, one_of, rx, tx).await
}

async fn internal_behavior<const GIRTH:usize>(context: SteadyContext, one_of: usize, rx: SteadyRx<Packet>, tx: SteadyTxBundle<Packet, { GIRTH }>) -> Result<(), Box<dyn Error>> {
    //info!("running {:?} {:?}",context.id(),context.name());
    let mut monitor = into_monitor!(context,[rx],tx);

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = rx.capacity().clone()/4;
    let _tx_girth = tx.len();

    while monitor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()) {

       // info!("router a");
        let _clean = wait_for_all_or_proceed_upon!(
            monitor.wait_periodic(Duration::from_millis(40)),
            monitor.wait_shutdown_or_avail_units(&mut rx,2),
            monitor.wait_shutdown_or_vacant_units_bundle(&mut tx,count/2,_tx_girth)
        );
       // info!("router b");

        let mut iter = monitor.take_into_iter(&mut rx);
        while let Some(t) = iter.next() {
            let index = (t.route as usize / one_of)  % tx.len();

         //   info!("name: {:?} one_of: {:?} block_size: {:?} route: {:?} index: {:?}", monitor.ident(), one_of, block_size, t.route, index);

            if let Err(e) = monitor.try_send(&mut tx[index], t) {
                  let _ = monitor.send_async(&mut tx[index], e, SendSaturation::IgnoreAndWait).await;
                  break;
            }
        }
    }
    Ok(())
}


#[cfg(test)]
mod router_tests {
    use std::time::Duration;
    use super::*;
    use async_std::test;
    use steady_state::Graph;


    // #[test]
    // pub(crate) async fn test_router() {
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
    //
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


