use std::error::Error;
use std::time::Duration;

#[allow(unused_imports)]
use log::*;
use steady_state::*;
use crate::actor::data_generator::Packet;

#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>) -> Result<(),Box<dyn Error>> {

    let mut monitor =  context.into_monitor([&rx], []);

    let mut rx = rx.lock().await;

    while monitor.is_running(&mut || rx.is_closed()) {
        monitor.wait_avail_units(&mut rx, 1).await;

        if let Some(packet) = monitor.try_take(&mut rx) {
            assert_eq!(packet.data.len(),128);
            //info!("data_router: {:?}", packet);
        }

        monitor.relay_stats_smartly().await;

    }
    Ok(())
}




#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>) -> Result<(),Box<dyn Error>> {

    let mut monitor = context.into_monitor([&rx], []);

    //guards for the channels, NOTE: we could share one channel across actors.
    let mut rx = rx.lock().await;

    loop {
        relay_test(&mut monitor, &mut rx).await;
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
                    if expected.eq(&measured) {
                        GraphTestResult::Ok(())
                    } else {
                        GraphTestResult::Err(format!("no match {:?} {:?}"
                                             ,expected
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