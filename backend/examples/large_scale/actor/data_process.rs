use std::error::Error;
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

    loop {
       let count = 3* (rx.capacity()/4);
       monitor.wait_avail_units(&mut rx, count).await;

           let mut max_now = tx.vacant_units();
           if max_now>0 {
               while max_now>0 {
                   if let Some(packet) = monitor.try_take(&mut rx) {
                       if let Err(e) = monitor.try_send(&mut tx,packet) {
                           error!("Error sending packet: {:?}",e);
                           break;
                       }
                       max_now -= 1;
                   } else {
                       break;
                   }
               }
               monitor.relay_stats_smartly().await;
           }

    }
    Ok(())
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