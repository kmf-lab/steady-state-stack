use std::error::Error;
use std::time::Duration;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::commander::SendOutcome;

use steady_state::SteadyRx;
use steady_state::SteadyTx;
use crate::actor::data_generator::Packet;

pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTx<Packet>) -> Result<(),Box<dyn Error>> {
    let cmd = context.into_monitor([&rx], [&tx]);

    internal_behavior(cmd, rx, tx).await
}

async fn internal_behavior<C:SteadyCommander>(mut cmd: C, rx: SteadyRx<Packet>, tx: SteadyTx<Packet>) -> Result<(), Box<dyn Error>> {

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = rx.capacity().min(tx.capacity()) / 2;
    while cmd.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()) {

        let _clean = await_for_all_or_proceed_upon!(
             cmd.wait_periodic(Duration::from_millis(20))
            ,cmd.wait_avail(&mut rx,count)
            ,cmd.wait_vacant(&mut tx,count)
        );

        let count = cmd.avail_units(&mut rx).min(cmd.vacant_units(&mut tx));
        if count > 0 {
            for _ in 0..count {
                if let Some(packet) = cmd.try_take(&mut rx) {
                    match cmd.try_send(&mut tx, packet) {
                        SendOutcome::Success => {}
                        SendOutcome::Blocked(packet) => {error!("Error sending packet: {:?}",packet); break;}
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
    fn test_process() {

        let mut graph = GraphBuilder::for_testing().build(());

        let bash_size = 100;
        let (approved_widget_in_tx, approved_widget_in_rx) = graph.channel_builder()
            .with_capacity(bash_size).build_channel();

        let (approved_widget_out_tx, approved_widget_out_rx) = graph.channel_builder()
            .with_capacity(bash_size).build_channel();

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn(move |context| internal_behavior(context
                                                          , approved_widget_in_rx.clone()
                                                          , approved_widget_out_tx.clone() ));

        graph.start();
        
        let test_data: Vec<Packet> = (0..bash_size).map(|i| Packet { route: i as u16, data: Default::default() }).collect();
        let _sent = approved_widget_in_tx.testing_send_all(test_data,true);
         sleep(Duration::from_millis(100));

        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(2));

        let expected = 0;
        assert_steady_rx_gt_count!(&approved_widget_out_rx,expected);
    }


}


