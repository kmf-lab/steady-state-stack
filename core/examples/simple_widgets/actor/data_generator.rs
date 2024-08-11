use std::error::Error;

#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::monitor::LocalMonitor;
use steady_state::SteadyRx;
use steady_state::{SteadyTx, Tx};
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
    use async_std::test;
    use steady_state::*;


    #[test]
    async fn test_generator() {
        //1. build test graph, the input and output channels and our actor
        let graph = Graph::new_test(());

        // let (approved_widget_tx_out,approved_widget_rx_out) = graph.channel_builder()
        //     .with_capacity(BATCH_SIZE).build();
        //
        // let state = InternalState {
        //     last_approval: None,
        //     buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
        // };
        //
        // graph.actor_builder()
        //     .with_name("UnitTest")
        //     .build_spawn(move |context| internal_behavior(context, approved_widget_rx_out.clone(), state));
        //
        // // //2. add test data to the input channels
        // let test_data: Vec<ApprovedWidgets> = (0..BATCH_SIZE).map(|i| ApprovedWidgets { original_count: 0, approved_count: i as u64 }).collect();
        // approved_widget_tx_out.testing_send(test_data, Duration::from_millis(30), true).await;
        //
        // // //3. run graph until the actor detects the input is closed
        // graph.start_as_data_driven(Duration::from_secs(240));
        //
        // //4. assert expected results
        // // TODO: not sure how to make this work.
        // //  println!("last approval: {:?}", &state.last_approval);
        // //  assert_eq!(approved_widget_rx_out.testing_avail_units().await, BATCH_SIZE);

    }



}



