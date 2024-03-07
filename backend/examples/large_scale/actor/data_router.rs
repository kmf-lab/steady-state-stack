use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use crate::actor::data_generator::Packet;

use futures::future::{FutureExt, pending};
use std::task::{Context, Poll};
use std::pin::Pin;


pub async fn run<const LEN:usize>(context: SteadyContext
                 , one_of: usize
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTxBundle<Packet,LEN>
                ) -> Result<(),Box<dyn Error>> {

    let mut monitor = context.into_monitor([&rx], SteadyBundle::tx_def_slice(&tx));

    let block_size = u16::MAX / one_of as u16;

    let mut rx_guard = rx.lock().await;
    let rx = &mut *rx_guard;

    while monitor.is_running(
        &mut || rx.is_empty() && rx.is_closed() && SteadyBundle::mark_closed(&tx)
    ) {

        monitor.wait_avail_units(rx,rx.capacity()/3).await; //TODO: need Timeout??

        while let Some(packet) = monitor.try_peek(rx) {
            let index = ((packet.route / block_size) as usize) % tx.len();
            let mut sender = tx[index].lock().await;
                let to_send = packet.clone();
                let _ = monitor.send_async(&mut *sender,to_send,true).await;
                let consumed = monitor.try_take(rx);
                assert!(consumed.is_some());
                monitor.relay_stats_smartly().await;

        }
        monitor.relay_stats_smartly().await;


    }
    Ok(())
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