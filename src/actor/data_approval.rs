use std::ops::DerefMut;
use std::sync::Arc;
use futures::lock::Mutex;
use crate::actor::data_generator::WidgetInventory;
use log::*;
use crate::steady::channel::SteadyRx;
use crate::steady::channel::SteadyTx;
use crate::steady::monitor::SteadyMonitor;
use crate::steady::monitor::LocalMonitor;

const BATCH_SIZE: usize = 2000;

#[derive(Clone, Debug, PartialEq, Copy)]
pub struct ApprovedWidgets {
    pub original_count: u64,
    pub approved_count: u64,
}

#[cfg(not(test))]
pub async fn run(monitor: SteadyMonitor
                 , rx: Arc<Mutex<SteadyRx<WidgetInventory>>>
                 , tx: Arc<Mutex<SteadyTx<ApprovedWidgets>>>) -> Result<(),()> {

    let mut tx_guard = tx.lock().await;
    let mut rx_guard = rx.lock().await;

    let tx = tx_guard.deref_mut();
    let rx = rx_guard.deref_mut();

    let mut monitor = monitor.init_stats(&mut[rx], &mut[tx]);
    let mut buffer = [WidgetInventory { count: 0, _payload: 0, }; BATCH_SIZE];

    loop {
        //in this example iterate once blocks/await until it has work to do
        //this example is a very responsive telemetry for medium load levels
        //single pass of work, do not loop in here
        if iterate_once(&mut monitor
                        , rx
                        , tx
                        , &mut buffer
        ).await {
            break Ok(());
        }
        //we relay all our telemetry and return to the top to block for more work.
        monitor.relay_stats_all().await;
    }
}


#[cfg(test)]
pub async fn run(monitor: SteadyMonitor
                 , rx: Arc<Mutex<SteadyRx<WidgetInventory>>>
                 , tx: Arc<Mutex<SteadyTx<ApprovedWidgets>>>) -> Result<(),()> {

      let mut rx_guard = rx.lock().await;
      let mut tx_guard = tx.lock().await;
      let rx = rx_guard.deref_mut();
      let tx = tx_guard.deref_mut();

      let mut monitor = monitor.init_stats(&mut[rx], &mut[tx]);
      let mut buffer = [WidgetInventory { count: 0, _payload: 0 }; BATCH_SIZE];

    loop {

        if iterate_once( &mut monitor
                         , rx
                         , tx
                         , &mut buffer
                         ).await {
            break;
        }


            monitor.relay_stats_all().await;
    }
    Ok(())
}

// important function break out to ensure we have a point to test on
async fn iterate_once<const R: usize, const T: usize>(monitor: &mut LocalMonitor<R, T>
                                                      , rx: & mut SteadyRx<WidgetInventory>
                                                      , tx: & mut SteadyTx<ApprovedWidgets>
                                                      , buf: &mut [WidgetInventory; BATCH_SIZE]) -> bool {


    if !rx.is_empty() {
        let count = monitor.take_slice(rx, buf);
        //TODO: need to re-use this space
        let mut approvals: Vec<ApprovedWidgets> = Vec::with_capacity(count);
        for b in buf.iter().take(count) {
            approvals.push(ApprovedWidgets {
                original_count: b.count,
                approved_count: b.count / 2,

            });
        }

        let sent = monitor.send_slice_until_full(tx, &approvals);
        //iterator of sent until the end
        let mut send = approvals.into_iter().skip(sent);
        for send_me in send {
            let _ = monitor.send_async(tx, send_me).await;
        }
    } else {

        //by design we wait here for new work
        match monitor.take_async(rx).await {
            Ok(m) => {
                let _ = monitor.send_async(tx, ApprovedWidgets {
                    original_count: m.count,
                    approved_count: m.count / 2,

                }).await;
            },
            Err(msg) => {
                error!("Unexpected error recv_async: {}",msg);
            }
        }
    }

    false
}


#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;
    use crate::steady::graph::SteadyGraph;

    #[test]
    async fn test_process() {
        crate::steady::util::util_tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_in, rx_in) = graph.channel_builder(8).build();
        let (tx_out, rx_out) = graph.channel_builder(8).build();

        let mock_monitor = graph.new_test_monitor("approval_monitor");

        let mut mock_monitor = mock_monitor.init_stats(&mut[], &mut[]);

        let mut tx_in_guard = tx_in.lock().await;
        let mut rx_in_guard = rx_in.lock().await;

        let mut tx_out_guard = tx_out.lock().await;
        let mut rx_out_guard = rx_out.lock().await;

        let tx_in = tx_in_guard.deref_mut();
        let rx_in = rx_in_guard.deref_mut();
        let tx_out = tx_out_guard.deref_mut();
        let rx_out = rx_out_guard.deref_mut();

        let _ = mock_monitor.send_async(tx_in, WidgetInventory {
            count: 5
            , _payload: 42
        }).await;
        let mut buffer = [WidgetInventory { count: 0, _payload: 0 }; BATCH_SIZE];

        let exit= iterate_once(&mut mock_monitor, rx_in, tx_out, &mut buffer ).await;
        assert_eq!(exit, false);

        let result = mock_monitor.take_async(rx_out).await.unwrap();
        assert_eq!(result.original_count, 5);
        assert_eq!(result.approved_count, 2);
    }

}