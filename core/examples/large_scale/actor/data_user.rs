use std::error::Error;

#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::{SteadyRx};
use crate::actor::data_generator::Packet;

#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, rx).await
}

async fn internal_behavior(context: SteadyContext, rx: SteadyRx<Packet>) -> Result<(), Box<dyn Error>> {
    //info!("running {:?} {:?}",context.id(),context.name());

    let mut monitor = into_monitor!(context,[rx], []);

    let mut rx = rx.lock().await;
    let mut _count = 0;

    while monitor.is_running(&mut || rx.is_closed_and_empty()) {
        wait_for_all!(monitor.wait_avail_units(&mut rx, 1)).await;

        while let Some(packet) = monitor.try_take(&mut rx) {
            assert_eq!(packet.data.len(), 62);
            _count += 1;
            //info!("data_router: {:?}", packet);

            //if _count % 1000_000 == 0 {
            //    info!("hello");
            //    panic!("go");
            //}
        }

        monitor.relay_stats_smartly();
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

    if let Some(simulator) = monitor.sidechannel_responder() {
        simulator.respond_with(|expected| {

            rx.block_until_not_empty(std::time::Duration::from_secs(20));
            match monitor.try_take(rx) {
                Some(measured) => {
                    let expected: &Packet = expected.downcast_ref::<Packet>().expect("error casting");
                    if expected.eq(&measured) {
                        Box::new("".to_string())
                    } else {
                        Box::new(format!("no match {:?} {:?}"
                                             ,expected
                                             ,measured).to_string())
                    }
                },
                None => Box::new("no data".to_string()),
            }

        }).await;
    }


}



#[cfg(test)]
mod tests {
    use async_std::test;
    use steady_state::Graph;

    #[test]
    async fn test_iterate_once() {


        let _graph = Graph::new("");


    }

}