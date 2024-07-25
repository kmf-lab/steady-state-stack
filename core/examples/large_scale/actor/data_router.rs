use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use crate::actor::data_generator::Packet;

//use futures::future::FutureExt;
use std::time::Duration;
use futures_util::lock::MutexGuard;

use steady_state::{Rx, SteadyRx};
use steady_state::{SteadyTxBundle, Tx};

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

    let block_size = u16::MAX / one_of as u16;

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    //let count = rx.capacity().clone()/4;
    //let _tx_girth = tx.len();

    while monitor.is_running(&mut || rx.is_closed_and_empty() && tx.mark_closed()) {
        wait_for_all_or_proceed_upon!(
            monitor.wait_periodic(Duration::from_millis(40)),
            monitor.wait_avail_units(&mut rx,2),
           // monitor.wait_vacant_units_bundle(&mut tx,count/2,_tx_girth)
        ).await;

        let mut iter = monitor.take_into_iter(&mut rx);
        while let Some(t) = iter.next() {
            let index = ((t.route / block_size) as usize) % tx.len();
            if let Err(e) = monitor.try_send(&mut tx[index], t) {
                let _ = monitor.send_async(&mut tx[index], e, SendSaturation::IgnoreAndWait).await;
                //   break;
            }
        }
        monitor.relay_stats_smartly();
    }
    Ok(())
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