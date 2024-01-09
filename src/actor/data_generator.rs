
use std::time::Duration;
use crate::args::Args;
use crate::steady::{SteadyTx, SteadyMonitor, LocalMonitor};

#[derive(Clone, Debug)]
pub struct WidgetInventory {
    pub(crate) count: u128
}

struct InternalState {
    pub(crate) count: u128
}

#[cfg(not(test))]
pub async fn run(monitor: SteadyMonitor
                 , opt: Args
                 , tx: SteadyTx<WidgetInventory> ) -> Result<(),()> {
    let mut monitor = monitor.init_stats(&[], &[&tx]);

    //keep long running state in here while you run
    let mut state = InternalState { count: 0 };
    loop {
        //single pass of work, do not loop in here
        if iterate_once(&mut monitor, &mut state, &tx).await {
            break Ok(());
        }
        //this is an example of an actor running periodically
        //we send telemetry and wait for the next time we are to run here
        monitor.relay_stats_periodic(Duration::from_millis(opt.gen_rate_ms)).await;
    }

}

#[cfg(test)]
pub async fn run(monitor: SteadyMonitor
                 , _opt: Args
                 , tx: SteadyTx<WidgetInventory>) -> Result<(),()> {
    let mut monitor = monitor.init_stats(&[], &[&tx]);

    loop {
         relay_test(&mut monitor, &tx).await;
         monitor.relay_stats_periodic(Duration::from_millis(30)).await;
   }
}
#[cfg(test)]

async fn relay_test(monitor: &mut LocalMonitor<0, 1>
                    , tx: &SteadyTx<WidgetInventory>) {
    use bastion::run;
    use bastion::message::MessageHandler;

    let ctx = monitor.ctx();
    MessageHandler::new(ctx.recv().await.unwrap())
        .on_question( |message: WidgetInventory, answer_sender| {
            run!(async {
                let _ = monitor.tx(&tx, message).await;
                answer_sender.reply("ok").unwrap();
               });
        });
}


async fn iterate_once<const R: usize,const T: usize>(monitor: &mut LocalMonitor<R, T>
                      , state: &mut InternalState
                      , tx_widget: &SteadyTx<WidgetInventory> ) -> bool
{
    let _ = monitor.tx(tx_widget, WidgetInventory {count: state.count.clone() }).await;
    state.count += 1;
    false
}


#[cfg(test)]
mod tests {
    use crate::actor::{WidgetInventory};
    use crate::actor::data_generator::{InternalState, iterate_once};
    use crate::steady::{SteadyGraph, SteadyTx};

    #[async_std::test]
    async fn test_iterate_once() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx, rx): (SteadyTx<WidgetInventory>, _) = graph.new_channel(8,&[]);
        let mock_monitor = graph.new_test_monitor("generator_monitor").await;
        let mut mock_monitor = mock_monitor.init_stats(&[], &[&tx]);
        let mut state = InternalState { count: 10 };

        let exit = iterate_once(&mut mock_monitor, &mut state, &tx).await;
        assert_eq!(exit, false);

        let record = mock_monitor.rx(&rx).await.unwrap();
        assert_eq!(record.count, 10);
    }

}