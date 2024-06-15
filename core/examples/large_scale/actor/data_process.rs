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
    //info!("running {:?} {:?}",context.id(),context.name());

    let mut monitor = into_monitor!(context, [rx], [tx]);

    //guards for the channels, NOTE: we could share one channel across actors.
    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = rx.capacity().min(tx.capacity()) /2;


    while monitor.is_running(&mut || rx.is_closed() && rx.is_empty() && tx.mark_closed()){

        let clean = wait_for_all_or_proceed_upon!(
             monitor.wait_periodic(Duration::from_millis(20))
            ,monitor.wait_avail_units(&mut rx,count)
            ,monitor.wait_vacant_units(&mut tx,count)
        ).await;

        let count = monitor.avail_units(&mut rx).min(monitor.vacant_units(&mut tx));
        if count>0 {
            single_iteration(&mut monitor, &mut rx, &mut tx, count);
            monitor.relay_stats_smartly();
        }


    }
    Ok(())
}

fn single_iteration(monitor: &mut LocalMonitor<1, 1>
                    , mut rx: &mut Rx<Packet>
                    , mut tx: &mut Tx<Packet>, count: usize) {
    for _ in 0..count {
        if let Some(packet) = monitor.try_take(&mut rx) {
            if let Err(e) = monitor.try_send(&mut tx, packet) {
                error!("Error sending packet: {:?}",e);
                break;
            }
        } else {
            break;
        }
    }
}


#[cfg(test)]
mod tests {

    use async_std::test;
    use steady_state::Graph;

    #[test]
    async fn test_iterate_once() {

        //util::logger::initialize();

        let _graph = Graph::new("");


    }

}