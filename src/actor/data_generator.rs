use std::sync::Arc;
use std::time::Duration;
use async_std::sync::Mutex;
use crate::args::Args;
use crate::steady::*;
use crate::steady::monitor::{LocalMonitor, SteadyMonitor};

#[derive(Clone, Debug, Copy)]
pub struct WidgetInventory {
    pub(crate) count: u128,
    pub(crate) payload: u128,
    pub(crate) extra: [u128;6],
}

struct InternalState {
    pub(crate) count: u128
}

#[cfg(not(test))]
pub async fn run(monitor: SteadyMonitor
                 , opt: Args
                 , tx: Arc<Mutex<SteadyTx<WidgetInventory>>> ) -> Result<(),()> {

    let mut tx_guard = guard!(tx);
    let tx = ref_mut!(tx_guard);

    let mut monitor = monitor.init_stats(&mut[], &mut[tx]);

    //keep long running state in here while you run
    let mut state = InternalState { count: 0 };
    loop {
        let mut multiplier = 400;   //100_000 per second
        //let mut multiplier = 120; //30_000 per second
        //40 every 4 ms -> 10_000

        loop {
            //single pass of work, do not loop in here
            iterate_once(&mut monitor, &mut state, tx).await;
            multiplier -= 1;
            if 0 == multiplier {
                break;
            }
        }
        //this is an example of an telemetry running periodically
        //we send telemetry and wait for the next time we are to run here
        monitor.relay_stats_periodic(Duration::from_millis(opt.gen_rate_ms)).await;
    }

}

#[cfg(test)]
pub async fn run(monitor: SteadyMonitor
                 , _opt: Args
                 , tx: Arc<Mutex<SteadyTx<WidgetInventory>>>) -> Result<(),()> {
    let mut tx_guard = guard!(tx);
    let tx = ref_mut!(tx_guard);
    let mut monitor = monitor.init_stats(&mut[], &mut[tx]);

    loop {
         relay_test(& mut monitor, tx).await;
         monitor.relay_stats_periodic(Duration::from_millis(30)).await;
   }
}
#[cfg(test)]

async fn relay_test(monitor: &mut LocalMonitor<0,1>
                    , tx: &mut SteadyTx<WidgetInventory>) {
    use bastion::run;
    use bastion::message::MessageHandler;

    if let Some(ctx) = monitor.ctx() {
        MessageHandler::new(ctx.recv().await.unwrap())
            .on_question(|message: WidgetInventory, answer_sender| {
                run!(async {
                    let _ = monitor.send_async(tx, message).await;
                    answer_sender.reply("ok").unwrap();
                   });
            });
    }
}


async fn iterate_once<const R: usize,const T: usize>(monitor: & mut LocalMonitor<R, T>
                      , state: &mut InternalState
                      , tx_widget: &mut SteadyTx<WidgetInventory> ) -> bool
{
    let _ = monitor.send_async(tx_widget, WidgetInventory {
        count: state.count.clone(),
        payload: 42,
        extra: [1,2,3,4,5,6]
    }).await;
    state.count += 1;
    false
}


#[cfg(test)]
mod tests {
    use crate::actor::data_generator::{InternalState, iterate_once};
    use crate::steady::SteadyGraph;

    #[async_std::test]
    async fn test_iterate_once() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx, rx) = graph.new_channel(8,&[]);
        let mock_monitor = graph.new_test_monitor("generator_monitor");

        let mut steady_tx_guard = guard!(tx);
        let mut steady_rx_guard = guard!(rx);
        let steady_tx = ref_mut!(steady_tx_guard);
        let steady_rx = ref_mut!(steady_rx_guard);

        let mut mock_monitor = mock_monitor.init_stats(&mut[steady_rx], &mut[steady_tx]);
        let mut state = InternalState { count: 10 };

        let exit = iterate_once(&mut mock_monitor, &mut state, steady_tx).await;
        assert_eq!(exit, false);

        let record = mock_monitor.take_async(steady_rx).await.unwrap();
        assert_eq!(record.count, 10);
    }

}