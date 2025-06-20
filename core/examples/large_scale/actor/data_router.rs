use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use crate::actor::data_generator::Packet;

//use futures::future::FutureExt;
use std::time::Duration;
use steady_state::steady_actor::SendOutcome;
use steady_state::SteadyRx;
use steady_state::SteadyTxBundle;

pub async fn run<const GIRTH:usize>(context: SteadyActorShadow
                                    , one_of: usize
                                    , rx: SteadyRx<Packet>
                                    , tx: SteadyTxBundle<Packet,GIRTH>
                ) -> Result<(),Box<dyn Error>> {

    internal_behavior(context.into_spotlight([&rx], tx.meta_data()), one_of, rx, tx).await
}

async fn internal_behavior<C: SteadyActor, const GIRTH:usize>(mut actor: C, one_of: usize, rx: SteadyRx<Packet>, tx: SteadyTxBundle<Packet, { GIRTH }>) -> Result<(), Box<dyn Error>> {
    

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = rx.capacity()/4;
    let tx_girth = tx.len();

    while actor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()) {

       // info!("router a");
        let _clean = await_for_all_or_proceed_upon!(
            actor.wait_periodic(Duration::from_millis(40)),
            actor.wait_avail(&mut rx,2),
            actor.wait_vacant_bundle(&mut tx,count/2,tx_girth)
        );
       // info!("router b");

        let iter = actor.take_into_iter(&mut rx);
        for t in iter {
            let index = (t.route as usize / one_of)  % tx.len();

         //   info!("name: {:?} one_of: {:?} block_size: {:?} route: {:?} index: {:?}", monitor.ident(), one_of, block_size, t.route, index);

            match actor.try_send(&mut tx[index], t) {
                SendOutcome::Success => {}
                SendOutcome::Blocked(t) => {actor.send_async(&mut tx[index], t, SendSaturation::AwaitForRoom).await;}
            }
        }
    }
    Ok(())
}


#[cfg(test)]
mod router_tests {
    use std::error::Error;
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::GraphBuilder;

    #[test]
    fn test_router() -> Result<(), Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(());

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

        graph.start();
        sleep(Duration::from_millis(60));
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(15))?;
        
    //     //4. assert expected results
    //     // TODO: not sure how to make this work.
    //     //  println!("last approval: {:?}", &state.last_approval);
    //     //  assert_eq!(approved_widget_rx_out.testing_avail_units().await, BATCH_SIZE);
        Ok(())
     }


}


