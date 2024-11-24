use std::error::Error;

#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::commander::SteadyCommander;
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
    internal_behavior(into_monitor!(context, [feedback], [tx]), feedback, tx).await
}

async fn internal_behavior<C:SteadyCommander>(mut cmd:C
                            , feedback: SteadyRx<ChangeRequest>
                            , tx: SteadyTx<WidgetInventory> ) -> Result<(),Box<dyn Error>> {

    let gen_rate_micros = if let Some(a) = cmd.args::<crate::Args>() {
        a.gen_rate_micros
    } else {
        10_000 //default
    };

    let mut feedback = feedback.lock().await;
    let mut tx = tx.lock().await;
    let mut count = 0;

    const MULTIPLIER:usize = 256;   //500_000 per second at 500 micros

    while cmd.is_running(&mut || tx.mark_closed() ) {

        let _clean = await_for_all!(cmd.wait_shutdown_or_vacant_units(&mut tx, MULTIPLIER));

        let len_out = tx.vacant_units().min(MULTIPLIER);

        let mut wids = Vec::with_capacity(len_out);

        (0..=len_out)
            .for_each(|num|
            wids.push(
                WidgetInventory {
                    count: count+num as u64,
                    _payload: 42,
                }));

        count+= len_out as u64;

        let _sent = cmd.send_slice_until_full(&mut tx, &wids);
 
        if let Some(feedback) = cmd.try_take(&mut feedback) {
              trace!("data_generator feedback: {:?}", feedback);
        }
        
        //this is an example of a telemetry running periodically
        //we send telemetry and wait for the next time we are to run here
        let _clean = cmd.relay_stats_periodic(std::time::Duration::from_micros(gen_rate_micros)).await;

    }
    Ok(())
}

#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<ChangeRequest>
                 , tx: SteadyTx<WidgetInventory>) -> Result<(),Box<dyn Error>> {

    let mut monitor = into_monitor!(context, [&rx], [&tx]);
    if let Some(responder) = monitor.sidechannel_responder() {

        let _rx = rx.lock().await;
        let mut tx = tx.lock().await;

        while monitor.is_running(&mut || tx.mark_closed() ) {
            let _responder = responder.respond_with(|message| {
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
    use futures_timer::Delay;
    use steady_state::*;
    use crate::actor::data_generator::internal_behavior;

    #[test]
    async fn test_generator() {
        let mut graph = GraphBuilder::for_testing().build(());
        
        let (feedback_tx_out,feedback_rx_out) = graph.channel_builder()
                                                                   .with_capacity(32)
                                                                   .build();

        const BATCH_SIZE:usize = 256;
        let (approved_widget_tx_out,approved_widget_rx_out) = graph.channel_builder()
                                                                   .with_capacity(BATCH_SIZE)
                                                                   .build();

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn(move |context| internal_behavior(context, feedback_rx_out.clone(), approved_widget_tx_out.clone()  ));
        
        graph.start();
       
        Delay::new(Duration::from_millis(500)).await;
        feedback_tx_out.testing_close(Duration::from_millis(10)).await;

        graph.request_stop();
        graph.block_until_stopped(Duration::from_secs(20));

        let t = approved_widget_rx_out.testing_take().await;
        assert_eq!(BATCH_SIZE, t.len());
      

    }



}



