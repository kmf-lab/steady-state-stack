use std::error::Error;

#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::steady_rx::RxMetaDataProvider;
use steady_state::SteadyRx;
use crate::actor::data_generator::Packet;

#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>) -> Result<(),Box<dyn Error>> {
    internal_behavior(context.into_monitor([&rx], []), rx).await
}

#[cfg(not(test))]
async fn internal_behavior<C: SteadyCommander>(mut cmd: C, rx: SteadyRx<Packet>) -> Result<(), Box<dyn Error>> {
    
    let mut rx = rx.lock().await;
    let mut _count = 0;
    while cmd.is_running(&mut || rx.is_closed_and_empty()) {

        //we only added two here to force the macro test of two items
        await_for_any!( cmd.wait_shutdown()
                       ,cmd.wait_avail(&mut rx, 1));

        while let Some(packet) = cmd.try_take(&mut rx) {
            assert_eq!(packet.data.len(), 62);
            _count += 1;

            //for testing panic capture
            // if _count % 1000_000 == 0 {
            //    info!("hello");
            //    panic!("go");
            // }
        }
    }
    Ok(())
}

#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<Packet>) -> Result<(),Box<dyn Error>> {

    let mut monitor = context.into_monitor( [&rx], []);

    if let Some(mut reponder) = monitor.sidechannel_responder() {

        //guards for the channels, NOTE: we could share one channel across actors.
        let mut rx = rx.lock().await;
        while monitor.is_running(&mut || rx.is_closed_and_empty()) {

            let clean = await_for_all!(
                  reponder.wait_available_units(1),
                  monitor.wait_avail(&mut rx, 1)
            );

            if clean  {
               // info!("user respond");
                reponder.respond_with(|expected| {
                    match monitor.try_take(&mut rx) {
                        Some(measured) => {

                            let expected: &Packet = expected.downcast_ref::<Packet>().expect("error casting");
                            //do not check the route id since it could be any random one of the users and is expected to not match
                            if expected.data.eq(&measured.data) {
                                Box::new("ok".to_string())
                            } else {
                                let failure = format!("no match {:?} {:?}"
                                                      , expected
                                                      , measured).to_string();
                                error!("failure: {}", failure);
                                Box::new(failure)
                            }

                        },
                        None => Box::new("no data, should await until rx has data before response ".to_string()),
                    }
                }).await;
               //TODO: add this feature  return Ok(()); //TODO: if we leave early do we get a vote on shutdown??  BUG to fix..
            }

            monitor.relay_stats_smartly();

        }
    }
    Ok(())
}


#[cfg(test)]
mod user_tests {
    use std::time::Duration;
    use futures_timer::Delay;
    use steady_state::GraphBuilder;

    #[async_std::test]
    async fn test_user() {
        let mut graph = GraphBuilder::for_testing().build(());
 
    //
    //     // let (approved_widget_tx_out, approved_widget_rx_out) = graph.channel_builder()
    //     //     .with_capacity(BATCH_SIZE).build();
    //     //
    //     // let state = InternalState {
    //     //     last_approval: None,
    //     //     buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
    //     // };
    //
    //     // graph.actor_builder()
    //     //     .with_name("UnitTest")
    //     //     .build_spawn(move |context| internal_behavior(context, approved_widget_rx_out.clone(), state));
    //     //
    //     // // //2. add test data to the input channels
    //     // let test_data: Vec<Packet> = (0..BATCH_SIZE).map(|i| Packet { original_count: 0, approved_count: i as u64 }).collect();
    //     // approved_widget_tx_out.testing_send(test_data, Duration::from_millis(30), true).await;
        
        graph.start();
        Delay::new(Duration::from_millis(60)).await;
        graph.request_stop();
        graph.block_until_stopped(Duration::from_secs(15));
        
    //     //4. assert expected results
    //     // TODO: not sure how to make this work.
    //     //  println!("last approval: {:?}", &state.last_approval);
    //     //  assert_eq!(approved_widget_rx_out.testing_avail_units().await, BATCH_SIZE);
     }


}
