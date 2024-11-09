use std::error::Error;
use std::time::Duration;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::{SteadyRx};
use steady_state::{SteadyTx};
use crate::actor::data_generator::Packet;

pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTx<Packet>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, rx, tx).await
}

async fn internal_behavior(context: SteadyContext, rx: SteadyRx<Packet>, tx: SteadyTx<Packet>) -> Result<(), Box<dyn Error>> {
    //info!("running {:?} {:?}",context.id(),context.name());

    let mut monitor = into_monitor!(context, [rx], [tx]);

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = rx.capacity().min(tx.capacity()) / 2;


    while monitor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()) {

        let _clean = wait_for_all_or_proceed_upon!(
             monitor.wait_periodic(Duration::from_millis(20))
            ,monitor.wait_shutdown_or_avail_units(&mut rx,count)
            ,monitor.wait_shutdown_or_vacant_units(&mut tx,count)
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
        }
    }
    Ok(())
}


#[cfg(test)]
mod process_tests {
    use std::time::Duration;
    use async_std::test;
    use futures_timer::Delay;
    use steady_state::GraphBuilder;
    use crate::actor::data_generator::Packet;
    use crate::actor::data_process::internal_behavior;

    #[test]
    pub(crate) async fn test_process() {

        let mut graph = GraphBuilder::for_testing().build(());

        let bash_size = 100;
        let (approved_widget_in_tx, approved_widget_in_rx) = graph.channel_builder()
            .with_capacity(bash_size).build();

        let (approved_widget_out_tx, approved_widget_out_rx) = graph.channel_builder()
            .with_capacity(bash_size).build();

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn(move |context| internal_behavior(context
                                                          , approved_widget_in_rx.clone()
                                                          , approved_widget_out_tx.clone() ));

        graph.start();
        
        let test_data: Vec<Packet> = (0..bash_size).map(|i| Packet { route: i as u16, data: Default::default() }).collect();
        let _sent = approved_widget_in_tx.testing_send_all(test_data,true);

        Delay::new(Duration::from_secs(2)).await;

        graph.request_stop();
        graph.block_until_stopped(Duration::from_secs(2));

        let _took = approved_widget_out_rx.testing_take().await;
        //println!("took: {:?}", took);


    }


}


