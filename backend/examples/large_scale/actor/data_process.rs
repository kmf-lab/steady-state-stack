use std::error::Error;
use std::time::Duration;
//use async_std::prelude::FutureExt;
use futures_util::lock::MutexGuard;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use crate::actor::data_generator::Packet;


#[allow(unreachable_code)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTx<Packet>) -> Result<(),Box<dyn Error>> {

    let mut monitor = context.into_monitor([&rx], [&tx]);

    //guards for the channels, NOTE: we could share one channel across actors.
    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = 3* (rx.capacity()/4);


    while monitor.is_running(&mut || rx.is_closed() && tx.mark_closed()){

    let timeout_duration = Duration::from_secs(5); // Example: 5 seconds timeout.

         match async_std::future::timeout(timeout_duration, monitor.wait_avail_units(&mut rx, count)).await {
                Ok(_) => {},
                Err(_) => {
                    monitor.relay_stats_smartly().await;
                    continue;
                }
         }

         monitor.wait_vacant_units(&mut tx, count).await;
         single_iteration(&mut monitor, &mut rx, &mut tx, count);
         monitor.relay_stats_smartly().await;

    }
    Ok(())
}

fn single_iteration(monitor: &mut LocalMonitor<1, 1>
                    , mut rx: &mut Rx<Packet>
                    , mut tx: &mut MutexGuard<Tx<Packet>>, count: usize) {
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
    use super::*;
    use async_std::test;
    use steady_state::Graph;

    /*
    #[test]
    async fn test_iterate_once() {

        util::logger::initialize();

        let mut graph = Graph::new("");


    }
//    */
}