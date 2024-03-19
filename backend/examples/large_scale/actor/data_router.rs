use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use crate::actor::data_generator::Packet;

use futures::future::FutureExt;

use std::time::Duration;
use futures_timer::Delay;
use futures_util::lock::MutexGuard;
use futures_util::select;


pub async fn run<const GIRTH:usize>(context: SteadyContext
                 , one_of: usize
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTxBundle<Packet,GIRTH>
                ) -> Result<(),Box<dyn Error>> {

    let mut monitor = context.into_monitor([&rx], tx.def_slice());

    let block_size = u16::MAX / one_of as u16;

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let count = rx.capacity().clone()/3;

    while monitor.is_running(&mut || rx.is_empty() && rx.is_closed() && tx.mark_closed()   ) {

        //we run atLeast once every 2 ms
        let delay_future = Delay::new(Duration::from_millis(2));
        select! {
            _ = delay_future.fuse() => {},
            _ = monitor.wait_avail_units(&mut rx,count).fuse() => {},
        }
        single_iteration(&mut monitor, block_size, &mut rx, &mut tx).await;
        monitor.relay_stats_smartly().await;

    }
    Ok(())
}

async fn single_iteration<const GIRTH:usize>(monitor: &mut LocalMonitor<1, GIRTH>
                                             , block_size: u16
                                             , mut rx: &mut MutexGuard<'_, Rx<Packet>>
                                             , tx: &mut Vec<MutexGuard<'_, Tx<Packet>>>) {
    while let Some(packet) = monitor.try_peek(&mut rx) {
        let index = ((packet.route / block_size) as usize) % tx.len();

        let to_send = packet.clone();
        let _ = monitor.send_async(&mut tx[index], to_send, true).await;
        let consumed = monitor.try_take(&mut rx);
        assert!(consumed.is_some());
        monitor.relay_stats_smartly().await;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;
    use steady_state::{Graph, util};

/*
    #[test]
    async fn test_process() {
        util::logger::initialize();
        let mut graph = Graph::new("");


    }
    //    */


}