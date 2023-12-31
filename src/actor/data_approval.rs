use crate::actor::data_generator::WidgetInventory;
use log::*;
use crate::steady::*;

#[derive(Clone, Debug, PartialEq)]
pub struct ApprovedWidgets {
    pub original_count: u128,
    pub approved_count: u128
}

#[cfg(not(test))]
pub async fn run(mut monitor: SteadyMonitor
                 , mut rx: SteadyRx<WidgetInventory>
                 , mut tx: SteadyTx<ApprovedWidgets>) -> Result<(),()> {
    loop {
        //in this example iterate once blocks/await until it has work to do
        //this example is a very responsive actor for medium load levels
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                         , &mut rx
                         , &mut tx).await {
            break Ok(());
        }
        //we relay all our telemetry and return to the top to block for more work.
        monitor.relay_stats_all().await;
    }
}

#[cfg(test)]
pub async fn run(mut monitor: SteadyMonitor, mut rx: SteadyRx<WidgetInventory>, mut tx: SteadyTx<ApprovedWidgets>) -> Result<(),()> {
    loop {
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                      , &mut rx
                      , &mut tx).await {
            break Ok(());
        }
        monitor.relay_stats_all().await;
    }
}

// important function break out to ensure we have a point to test on
async fn iterate_once(monitor: &mut SteadyMonitor
                 , rx: &SteadyRx<WidgetInventory>
                 , tx: &SteadyTx<ApprovedWidgets>) -> bool  {

    //by design we wait here for new work
     match monitor.rx(rx).await {
        Ok(m) => {
            monitor.tx(tx, ApprovedWidgets {
                original_count: m.count,
                approved_count: m.count/2
            }).await;
        },
        Err(e) => {
            error!("Unexpected error recv_async: {}",e);
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
        let (tx_in, rx_in): (SteadyTx<WidgetInventory>, _) = graph.new_channel(8);
        let (tx_out, rx_out): (SteadyTx<ApprovedWidgets>, _) = graph.new_channel(8);

        let mut mock_monitor = graph.new_test_monitor("approval_monitor").await;

        mock_monitor.tx(&tx_in, WidgetInventory {count: 5 }).await;

        let exit= iterate_once(&mut mock_monitor, &rx_in, &tx_out).await;
        assert_eq!(exit, false);

        let result = mock_monitor.rx(&rx_out).await.unwrap();
        assert_eq!(result.original_count, 5);
        assert_eq!(result.approved_count, 2);
    }

}