#[allow(unused_imports)]
use log::*;
#[allow(unused_imports)]
use std::time::Duration;
use steady_state::*;
use std::error::Error;
use steady_state::steady_actor::SendOutcome;

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub(crate) struct PrintSignal {
   pub(crate) tick: u32
}

pub async fn run(actor: SteadyActorShadow, print_signal_tx: SteadyTx<PrintSignal>) -> Result<(),Box<dyn Error>> {
    let actor = actor.into_spotlight([], [&print_signal_tx]);

    if actor.use_internal_behavior {
        internal_behavior(actor, print_signal_tx).await
    } else {
       actor.simulated_behavior(sim_runners!(print_signal_tx)).await
    }
}

async fn internal_behavior<A: SteadyActor>(mut actor: A
                                           , print_signal_tx: SteadyTx<PrintSignal>) -> Result<(),Box<dyn Error>> {

    let mut print_signal_tx = print_signal_tx.lock().await;
    let mut tick = 0;
    while actor.is_running(&mut || i!(print_signal_tx.mark_closed())) {
         let clean = await_for_any!(actor.wait_periodic(Duration::from_secs(2)));
         if clean {
             tick += 1;
             match actor.try_send(&mut print_signal_tx, PrintSignal { tick }) {
                 SendOutcome::Success => {}
                 SendOutcome::Blocked(signal) => {error!("channel backed up, failed to send tick: {:?}",signal.tick);}
                 SendOutcome::Timeout(signal) => {error!("timeout sending tick: {:?}",signal.tick);}
                 SendOutcome::Closed(_) => break,
             }
             actor.relay_stats();
         }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::thread::sleep;
    use std::time::Duration;
    use steady_state::*;
    use super::*;

    #[test]
    fn test_timer_actor() -> Result<(),Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(());
        let (print_signal_tx,test_print_signal_rx) = graph.channel_builder().with_capacity(4).build_channel();
        graph.actor_builder()
            .with_name("UnitTest")
            .build( move |context|
                internal_behavior(context,print_signal_tx.clone()), SoloAct
            );
        graph.start(); //startup the graph
        sleep(Duration::from_secs(5));
        graph.request_shutdown(); //our actor has no input so it immediately stops upon this request
        graph.block_until_stopped(Duration::from_secs(1))?;

        let expected = 2;
        assert_steady_rx_eq_count!(&test_print_signal_rx,expected);
        Ok(())
    }
}
