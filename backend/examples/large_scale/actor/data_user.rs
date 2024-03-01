use std::ops::DerefMut;
use std::time::Duration;

#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::monitor::LocalMonitor;
use crate::actor::data_generator::Packet;

#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>) -> Result<(),()> {

    let mut monitor =  context.into_monitor([&rx], []);


    let mut rx_guard = rx.lock().await;
    let rx = rx_guard.deref_mut();

    loop {
        if let Ok(packet) = monitor.take_async(rx).await {
            assert_eq!(packet.data.len(),128);
            //info!("data_router: {:?}", packet);
        }
        monitor.relay_stats_smartly().await;

    }
}




#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>) -> Result<(),()> {

    let mut monitor = context.into_monitor([&rx], []);

    //guards for the channels, NOTE: we could share one channel across actors.
    let mut rx_guard = rx.lock().await;
    let rx = rx_guard.deref_mut();

    loop {
        relay_test(&mut monitor, rx).await;
        if false {
            break;
        }
    }
    Ok(())
}


#[cfg(test)]
async fn relay_test(monitor: &mut LocalMonitor<1, 0>, rx: &mut Rx< Packet>) {

    if let Some(simulator) = monitor.edge_simulator() {
        simulator.respond_to_request(|expected: Packet| {

            rx.block_until_not_empty(Duration::from_secs(20));
            match monitor.try_take(rx) {
                Some(measured) => {
                    let result = expected.cmp(&measured);
                    if result.is_eq() {
                        GraphTestResult::Ok(())
                    } else {
                        GraphTestResult::Err(format!("no match {:?} {:?} {:?}"
                                             ,expected
                                             ,result
                                             ,measured))
                    }
                },
                None => GraphTestResult::Err("no data".to_string()),
            }

        }).await;
    }


}



#[cfg(test)]
mod tests {
    use async_std::test;
    /*
    #[test]
    async fn test_iterate_once() {

        util::logger::initialize();

        let mut graph = Graph::new("");


    }
//    */
}