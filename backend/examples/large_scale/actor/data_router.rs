use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use crate::actor::data_generator::Packet;

//use futures::future::FutureExt;
use std::time::Duration;
use futures_util::lock::MutexGuard;
use num_traits::Zero;
use tide::Middleware;

pub async fn run<const GIRTH:usize>(context: SteadyContext
                 , one_of: usize
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTxBundle<Packet,GIRTH>
                ) -> Result<(),Box<dyn Error>> {

   //info!("running {:?} {:?}",context.id(),context.name());
    let mut monitor = into_monitor!(context,[rx],tx);

    let block_size = u16::MAX / one_of as u16;

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = rx.capacity().clone()/3;
    let bsize = tx.len();

    while monitor.is_running(&mut || rx.is_empty() && rx.is_closed() && tx.mark_closed()   ) {

        wait_for_all_or_proceed_upon!(
            monitor.wait_periodic(Duration::from_millis(40)),
            monitor.wait_avail_units(&mut rx,count),
            monitor.wait_vacant_units_bundle(&mut tx,count/2,bsize)
        ).await;

        while let Some(t) = monitor.try_take(&mut rx) {
            let index = ((t.route / block_size) as usize) % tx.len();
            if let Err(e) =  monitor.try_send(&mut tx[index], t) {
                let _ = monitor.send_async(&mut tx[index], e, SendSaturation::IgnoreAndWait).await;
                break;
            }

        }
        monitor.relay_stats_smartly();

    }
    Ok(())
}


//this unused block shows how you can peek and do processing then send
//so you can ensure no packet is ever lost even upon panic.
//NOTE: this adds cost due to the required clone()
async fn single_iteration<const GIRTH:usize>(monitor: &mut LocalMonitor<1, GIRTH>
                                             , block_size: u16
                                             , mut rx: &mut MutexGuard<'_, Rx<Packet>>
                                             , tx: &mut Vec<MutexGuard<'_, Tx<Packet>>>) {

        while let Some(peek_packet_ref) = monitor.try_peek(&mut rx) {
            let index = ((peek_packet_ref.route / block_size) as usize) % tx.len();

            let new_copy = peek_packet_ref.clone();
            let _ = monitor.send_async(&mut tx[index], new_copy, SendSaturation::IgnoreAndWait).await;
            let consumed = monitor.try_take(&mut rx);
            assert!(consumed.is_some());
        }
    monitor.relay_stats_smartly();
}


#[cfg(test)]
mod tests {

    use async_std::test;
    use steady_state::Graph;


    #[test]
    async fn test_process() {
       // util::logger::initialize();
        let _graph = Graph::new("");


    }



}