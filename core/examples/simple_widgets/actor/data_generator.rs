use std::error::Error;

#[allow(unused_imports)]
use log::*;
use steady_state::*;
use crate::actor::data_feedback::ChangeRequest;

#[derive(Clone, Debug, Copy)]
pub struct WidgetInventory {
    pub(crate) count: u64,
    pub(crate) _payload: u64,
}



#[cfg(not(test))]
pub async fn run(context: SteadyContext
                               , feedback: SteadyRx<ChangeRequest>
                               , tx: SteadyTx<WidgetInventory> ) -> Result<(),Box<dyn Error>> {
    internal_behavior(context, feedback, tx).await
}

#[derive(Default,Clone)]
struct InternalState {
    count: u64
}

pub async fn internal_behavior(context: SteadyContext
                                , feedback: SteadyRx<ChangeRequest>
                                , tx: SteadyTx<WidgetInventory> ) -> Result<(),Box<dyn Error>> {

    let gen_rate_micros = if let Some(a) = context.args::<crate::Args>() {
        a.gen_rate_micros
    } else {
        10_000 //default
    };

    let mut monitor = into_monitor!(context, [feedback], [tx]);
    let mut feedback = feedback.lock().await;
    let mut tx = tx.lock().await;

    const MULTIPLIER:usize = 256;   //500_000 per second at 500 micros

    let mut state = InternalState {
        count: 0,
    };
    while monitor.is_running(&mut || tx.mark_closed() ) {
        
        
        wait_for_all!(monitor.wait_shutdown_or_vacant_units(&mut tx, MULTIPLIER));
        

        let mut wids = Vec::with_capacity(MULTIPLIER);

        (0..=MULTIPLIER)
            .for_each(|num|
            wids.push(
                WidgetInventory {
                    count: state.count+num as u64,
                    _payload: 42,
                }));

        state.count+= MULTIPLIER as u64;


        let sent = monitor.send_slice_until_full(&mut tx, &wids);
        //iterator of sent until the end
        let consume = wids.into_iter().skip(sent);
        for send_me in consume {
            let _ = monitor.send_async(&mut tx, send_me, SendSaturation::Warn).await;
        }


        if let Some(feedback) = monitor.try_take(&mut feedback) {
              trace!("data_generator feedback: {:?}", feedback);
        }

        //this is an example of a telemetry running periodically
        //we send telemetry and wait for the next time we are to run here
        let _clean = monitor.relay_stats_periodic(std::time::Duration::from_micros(gen_rate_micros)).await;

    }
    Ok(())
}

#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<ChangeRequest>
                 , tx: SteadyTx<WidgetInventory>) -> Result<(),Box<dyn Error>> {

    let mut monitor = into_monitor!(context, [&rx], [&tx]);
    if let Some(responder) = monitor.sidechannel_responder() {

        let mut _rx = rx.lock().await;
        let mut tx = tx.lock().await;

        while monitor.is_running(&mut || tx.mark_closed() ) {
            let responder = responder.respond_with(|message| {
                let msg: &WidgetInventory = message.downcast_ref::<WidgetInventory>().expect("error casting");
                match monitor.try_send(&mut tx, msg.clone()) {
                    Ok(()) => Box::new("ok".to_string()),
                    Err(m) => Box::new(m),
                }
            }).await;
            monitor.relay_stats_smartly();
        }
    }
    Ok(())
}

#[cfg(test)]
mod generator_tests {
    use std::time::Duration;
    use async_std::test;
    use steady_state::*;
    use crate::actor::data_generator::{internal_behavior, InternalState};

    #[test]
    async fn test_generator() {
        // //1. build test graph, the input and output channels and our actor
         let mut graph = GraphBuilder::for_testing().build(());
        //
        // let (approved_widget_tx_out,approved_widget_rx_out) = graph.channel_builder()
        //     .with_capacity(256).build();
        //
        // let state = InternalState::default();
        //
        // graph.actor_builder()
        //     .with_name("UnitTest")
        //     .build_spawn(move |context| internal_behavior(context, approved_widget_rx_out.clone()));
        //
        // graph.start();
        // graph.request_stop();
        //
        // //
        // // // //2. add test data to the input channels
        // // let test_data: Vec<ApprovedWidgets> = (0..BATCH_SIZE).map(|i| ApprovedWidgets { original_count: 0, approved_count: i as u64 }).collect();
        // // approved_widget_tx_out.testing_send(test_data, Duration::from_millis(30), true).await;
        // //
        //
        // graph.block_until_stopped(Duration::from_secs(10));
        // //
        // // //4. assert expected results
        // // // TODO: not sure how to make this work.
        // // //  println!("last approval: {:?}", &state.last_approval);
        // // //  assert_eq!(approved_widget_rx_out.testing_avail_units().await, BATCH_SIZE);

    }



}



