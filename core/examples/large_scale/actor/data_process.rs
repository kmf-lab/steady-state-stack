use std::error::Error;
use std::time::Duration;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::steady_actor::SendOutcome;

use steady_state::SteadyRx;
use steady_state::SteadyTx;
use crate::actor::data_generator::Packet;

pub async fn run(context: SteadyActorShadow
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTx<Packet>) -> Result<(),Box<dyn Error>> {
    let actor = context.into_spotlight([&rx], [&tx]);
    if actor.use_internal_behavior {
        internal_behavior(actor, rx, tx).await
    } else {
        actor.simulated_behavior(sim_runners!(rx, tx)).await
    }
}

async fn internal_behavior<C: SteadyActor>(mut actor: C, rx: SteadyRx<Packet>, tx: SteadyTx<Packet>) -> Result<(), Box<dyn Error>> {

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = rx.capacity().min(tx.capacity()) / 2;
    while actor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()) {

        let _clean = await_for_all_or_proceed_upon!(
             actor.wait_periodic(Duration::from_millis(20))
            ,actor.wait_avail(&mut rx,count)
            ,actor.wait_vacant(&mut tx,count)
        );

        let count = actor.avail_units(&mut rx).min(actor.vacant_units(&mut tx));
        if count > 0 {
            for _ in 0..count {
                if let Some(packet) = actor.try_take(&mut rx) {
                    match actor.try_send(&mut tx, packet) {
                        SendOutcome::Success => {}
                        SendOutcome::Blocked(packet) => {error!("Error sending packet: {:?}",packet); break;}
                        SendOutcome::Timeout(packet) => {error!("Timeout sending packet: {:?}",packet); break;}
                        SendOutcome::Closed(_) => break,
                    }
                } else {
                    error!("Error reading packet");
                    break;
                }
            }
        }
    }
    Ok(())
}


#[cfg(test)]
mod process_tests {
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::*;
    use crate::actor::data_generator::Packet;
    use crate::actor::data_process::internal_behavior;

    #[test]
    fn test_process() -> Result<(), Box<dyn Error>>{

        let mut graph = GraphBuilder::for_testing().build(());

        let bash_size = 100;
        let (approved_widget_in_tx, approved_widget_in_rx) = graph.channel_builder()
            .with_capacity(bash_size).build_channel();

        let (approved_widget_out_tx, approved_widget_out_rx) = graph.channel_builder()
            .with_capacity(bash_size).build_channel();

        graph.actor_builder()
            .with_name("UnitTest")
            .build(move |context| internal_behavior(context
                                                          , approved_widget_in_rx.clone()
                                                          , approved_widget_out_tx.clone() ), SoloAct);

        graph.start();
        
        let test_data: Vec<Packet> = (0..bash_size).map(|i| Packet { route: i as u16, data: Default::default() }).collect();
        let _sent = approved_widget_in_tx.testing_send_all(test_data,true);
         sleep(Duration::from_millis(100));

        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(2))?;

        let expected = 0;
        assert_steady_rx_gt_count!(&approved_widget_out_rx,expected);
        Ok(())
    }


}
