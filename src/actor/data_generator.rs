use std::sync::Arc;
use std::time::Duration;
use futures::lock::Mutex;
use crate::args::Args;
use crate::steady::*;
use crate::steady::monitor::{LocalMonitor, SteadyMonitor};

#[derive(Clone, Debug, Copy)]
pub struct WidgetInventory {
    pub(crate) count: u64,
    pub(crate) _payload: u64,
}



#[derive(Clone, Debug, Copy)]
struct InternalState {
    pub(crate) count: u64,

}

#[cfg(not(test))]
pub async fn run(monitor: SteadyMonitor
                 , opt: Args
                 , tx: Arc<Mutex<SteadyTx<WidgetInventory>>> ) -> Result<(),()> {

    let mut tx_guard = guard!(tx);
    let tx = ref_mut!(tx_guard);

    const MULTIPLIER:usize = 1000;   //2_000_000 per second at 500 micros

    let mut monitor = monitor.init_stats(&mut[], &mut[tx]);

    //keep long running state in here while you run

    let mut state = InternalState {
        count: 0,
    };
    loop {

         //single pass of work, do not loop in here
        iterate_once(&mut monitor, &mut state, tx, MULTIPLIER).await;

        //this is an example of an telemetry running periodically
        //we send telemetry and wait for the next time we are to run here
        monitor.relay_stats_periodic(Duration::from_micros(opt.gen_rate_micros)).await;
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
         monitor.relay_stats_periodic(Duration::from_micros(1000*30)).await;
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
                      , tx_widget: &mut SteadyTx<WidgetInventory>
    , multiplier: usize
                    ) -> bool
{
    let mut wids = Vec::with_capacity(multiplier);

    (0..=multiplier)
        .for_each(|num|
            wids.push(
                WidgetInventory {
                count: state.count+num as u64,
                _payload: 42,
        }));

    state.count+= multiplier as u64;

    let sent = monitor.send_slice_until_full(tx_widget, &wids);
    //iterator of sent until the end
    let mut consume = wids.into_iter().skip(sent);
    for send_me in consume {
        let _ = monitor.send_async(tx_widget, send_me).await;
    }

    false
}


#[cfg(test)]
mod tests {
    use crate::actor::data_generator::{InternalState, iterate_once};
    use crate::actor::WidgetInventory;
    use crate::steady::SteadyGraph;

  //  #[async_std::test]
    async fn test_iterate_once() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx, rx) = graph.new_channel(5,&[]);
        let mock_monitor = graph.new_test_monitor("generator_monitor");

        let mut steady_tx_guard = guard!(tx);
        let mut steady_rx_guard = guard!(rx);
        let steady_tx = ref_mut!(steady_tx_guard);
        let steady_rx = ref_mut!(steady_rx_guard);

        let mut mock_monitor = mock_monitor.init_stats(&mut[steady_rx], &mut[steady_tx]);
        let mut state = InternalState {
            count: 10,
        };
        let exit = iterate_once(&mut mock_monitor, &mut state, steady_tx,1).await;
        assert_eq!(exit, false);

        let record = mock_monitor.take_async(steady_rx).await.unwrap();
        assert_eq!(record.count, 10);
    }

}