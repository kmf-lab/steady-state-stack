use futures::future;
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
        iterate_once( &mut monitor
                 , &mut rx
                 , &mut tx).await;
        monitor.relay_stats_all().await;
    }
    future::pending().await
}

#[cfg(test)]
pub async fn behavior(mut monitor: SteadyMonitor, mut rx: SteadyRx<WidgetInventory>, mut tx: SteadyTx<ApprovedWidgets>) -> Result<(),()> {
    loop {
        //single pass of work, do not loop in here
        iterate_once( &mut monitor
                      , &mut rx
                      , &mut tx).await;
        monitor.relay_stats_all().await;
    }
    unreachable!("This code should never be reached otherwise Ok(()) should have been returned")

}

// important function break out to ensure we have a point to test on
async fn iterate_once(monitor: &mut SteadyMonitor
                 , rx: &mut SteadyRx<WidgetInventory>
                 , tx: &mut SteadyTx<ApprovedWidgets>)  {

    //by design we wait here for new work
     match rx.rx(monitor).await {
        Ok(m) => {
            tx.tx(monitor, ApprovedWidgets {
                original_count: m.count,
                approved_count: m.count/2
            }).await;
        },
        Err(e) => {
            error!("Unexpected error recv_async: {}",e);
        }
    }

}


#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;

    #[test]
    async fn test_process() {






    }

}