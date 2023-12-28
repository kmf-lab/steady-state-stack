use crate::actor::data_generator::WidgetInventory;
use log::*;
use crate::steady::*;

#[derive(Clone, Debug)]
pub struct ApprovedWidgets {
    pub original_count: u128,
    pub approved_count: u128
}

#[cfg(not(test))]
pub async fn behavior(mut monitor: SteadyMonitor
                      , mut rx: SteadyRx<WidgetInventory>
                      , mut tx: SteadyTx<ApprovedWidgets>) -> Result<(),()> {
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

#[cfg(test)]
pub async fn behavior(mut monitor: SteadyMonitor, mut rx: SteadyRx<WidgetInventory>, mut tx: SteadyTx<ApprovedWidgets>) -> Result<(),()> {
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
    use flexi_logger::{Logger, LogSpecification};

    #[test]
    async fn test_process() {


        let _ = Logger::with(LogSpecification::env_or_parse("info").unwrap())
            .format(flexi_logger::colored_with_thread)
            .start();

        let mut graph = SteadyGraph::new();
        let (tx_in, rx_in): (SteadyTx<WidgetInventory>, _) = graph.new_channel(8);
        let (tx_out, rx_out): (SteadyTx<ApprovedWidgets>, _) = graph.new_channel(8);

        let mut monitor = graph.new_monitor().await.wrap("test",None);

        let exit= iterate_once(&mut monitor, &rx_in, &tx_out).await;
        assert_eq!(exit, false);

        // TODO: need more testing

    }

}