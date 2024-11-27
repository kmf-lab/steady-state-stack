
#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use std::error::Error;
#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub(crate) struct PrintSignal {
   pub(crate) tick: u32
}

#[cfg(not(test))]
pub async fn run(context: SteadyContext
        ,print_signal_tx: SteadyTx<PrintSignal>) -> Result<(),Box<dyn Error>> {
  internal_behavior(into_monitor!(context, [],[print_signal_tx]),print_signal_tx).await
}

async fn internal_behavior<C:SteadyCommander>(mut cmd: C
        ,print_signal_tx: SteadyTx<PrintSignal>) -> Result<(),Box<dyn Error>> {

    let mut print_signal_tx = print_signal_tx.lock().await;
    let mut tick = 0;
    while cmd.is_running(&mut || print_signal_tx.mark_closed()) {
         let clean = await_for_any!(cmd.wait_periodic(Duration::from_secs(2)));
         if clean {
             tick += 1;
             if let Err(t) = cmd.try_send(&mut print_signal_tx, PrintSignal { tick }) {
                 error!("channel backed up, failed to send tick: {:?}",t.tick);
             };
             cmd.relay_stats();
         }
    }
    Ok(())
}


#[cfg(test)]
pub async fn run(context: SteadyContext
                 ,print_signal_tx: SteadyTx<PrintSignal>
) -> Result<(),Box<dyn Error>> {
    let mut cmd =  into_monitor!(context, [],[print_signal_tx]);
    if let Some(responder) = cmd.sidechannel_responder() {
        let mut print_signal_tx = print_signal_tx.lock().await;
        while cmd.is_running(&mut ||print_signal_tx.mark_closed()) {
            // in main use graph.sidechannel_director node_call(msg,"TimerActor")
            let _did_echo = responder.echo_responder(&mut cmd,&mut print_signal_tx).await;
        }
    }
    Ok(())
}



#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;
    use futures_timer::Delay;
    use steady_state::*;
    use super::*;

    #[async_std::test]
    pub(crate) async fn test_simple_process() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (print_signal_tx,test_print_signal_rx) = graph.channel_builder().with_capacity(4).build();
        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context|
                internal_behavior(context,print_signal_tx.clone())
            );
        graph.start(); //startup the graph

        //this actor produces one message every 2 seconds
        Delay::new(Duration::from_secs(5)).await;

        graph.request_stop(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(1));

        assert_eq!(test_print_signal_rx.testing_avail_units().await, 2);
    }

}