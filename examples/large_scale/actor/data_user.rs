use std::ops::DerefMut;

#[allow(unused_imports)]
use log::*;
use steady_state::*;
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
        monitor.relay_stats_all().await;

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
    use bastion::prelude::*;
/*
    if let Some(ctx) = monitor.ctx() {
        MessageHandler::new(ctx.recv().await.unwrap())
            .on_question(|expected: ApprovedWidgets, answer_sender| {
                run!(async {
                let recevied = monitor.take_async(rx).await.unwrap();
                answer_sender.reply(if expected == recevied {"ok"} else {"err"}).unwrap();
               });
            });
    }

 */
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