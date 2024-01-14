use std::sync::Arc;
use async_std::sync::Mutex;
use crate::actor::data_generator::WidgetInventory;
use log::*;
use crate::steady::*;
use SteadyMonitor;

#[derive(Clone, Debug, PartialEq)]
pub struct ApprovedWidgets {
    pub original_count: u128,
    pub approved_count: u128
}

#[cfg(not(test))]
pub async fn run(monitor: SteadyMonitor
                 , rx: Arc<Mutex<SteadyRx<WidgetInventory>>>
                 , tx: Arc<Mutex<SteadyTx<ApprovedWidgets>>>) -> Result<(),()> {

    let mut tx_guard = guard!(tx);
    let mut rx_guard = guard!(rx);

    let tx = ref_mut!(tx_guard);
    let rx = ref_mut!(rx_guard);


    let mut monitor = monitor.init_stats(&mut[rx], &mut[tx]);

    loop {
        //in this example iterate once blocks/await until it has work to do
        //this example is a very responsive actor for medium load levels
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                         , rx
                         , tx).await {
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

      let mut rx_guard = guard!(rx);
      let mut tx_guard = guard!(tx);
      let rx = ref_mut!(rx_guard);
      let tx = ref_mut!(tx_guard);

      let mut monitor = monitor.init_stats(&mut[rx], &mut[tx]);

    loop {
                match monitor.take_async(rx).await {
                    Ok(m) => {
                        let _ = monitor.send_async(tx, ApprovedWidgets {
                            original_count: m.count,
                            approved_count: m.count / 2
                        }).await;
                    },
                    Err(msg) => {
                        error!("Unexpected error recv_async: {}",msg);
                    }
                }
            monitor.relay_stats_all().await;
    }
    Ok(())
}

// important function break out to ensure we have a point to test on
async fn iterate_once<const R: usize, const T: usize>(monitor: &mut LocalMonitor<R, T>
                                                                  , rx: & mut SteadyRx<WidgetInventory>
                                                                  , tx: & mut SteadyTx<ApprovedWidgets>) -> bool {


    //by design we wait here for new work
     match monitor.take_async(rx).await {
        Ok(m) => {
            let _ = monitor.send_async(tx, ApprovedWidgets {
                original_count: m.count, approved_count: m.count/2
            }).await;
        },
        Err(msg) => {
            error!("Unexpected error recv_async: {}",msg);
        }
    }
    false

}


#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;

    #[test]
    async fn test_process() {
        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_in, rx_in) = graph.new_channel(8,&[]);
        let (tx_out, rx_out) = graph.new_channel(8,&[]);

        let mock_monitor = graph.new_test_monitor("approval_monitor");

        let mut mock_monitor = mock_monitor.init_stats(&mut[], &mut[]);

        let mut tx_in_guard = guard!(tx_in);
        let mut rx_in_guard = guard!(rx_in);

        let mut tx_out_guard = guard!(tx_out);
        let mut rx_out_guard = guard!(rx_out);

        let tx_in = ref_mut!(tx_in_guard);
        let rx_in = ref_mut!(rx_in_guard);
        let tx_out = ref_mut!(tx_out_guard);
        let rx_out = ref_mut!(rx_out_guard);

        let _ = mock_monitor.send_async(tx_in, WidgetInventory {count: 5 }).await;

        let exit= iterate_once(&mut mock_monitor, rx_in, tx_out).await;
        assert_eq!(exit, false);

        let result = mock_monitor.take_async(rx_out).await.unwrap();
        assert_eq!(result.original_count, 5);
        assert_eq!(result.approved_count, 2);
    }

}