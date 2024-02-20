use std::ops::DerefMut;
use log::*;
use steady_state::*;
use crate::actor::data_generator::Packet;

pub async fn run<const LEN:usize>(context: SteadyContext
                 , one_of: usize
                 , rx: SteadyRx<Packet>
                 , tx: SteadyTxBundle<Packet,LEN>
                ) -> Result<(),()> {

    let mut monitor = context.into_monitor([&rx], SteadyBundle::tx_def_slice(&tx));

    let block_size = u16::MAX / one_of as u16;

    let mut rx_guard = rx.lock().await;
    let rx = rx_guard.deref_mut();

    loop {

        monitor.wait_avail_units(rx,rx.capacity().get()/3).await; //TODO: need Timeout??

        while let Some(packet) = monitor.try_take(rx) {
            let index = ((packet.route / block_size) as usize) % tx.len();
            let mut lock = tx[index].lock().await;
            let _ = monitor.send_async(lock.deref_mut(),packet).await;
        }

        monitor.relay_stats_all().await;

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