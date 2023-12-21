
use futures::select;
use crate::actor::data_generator::WidgetInventory;
use log::*;
use crate::steady::*;
use futures::FutureExt;

#[derive(Clone, Debug)]
pub struct ApprovedWidgets {
    original_count: u128,
    approved_count: u128
}

#[cfg(not(test))]
pub async fn behavior(mut monitor: SteadyMonitor, mut rx: SteadyRx<WidgetInventory>, mut tx: SteadyTx<ApprovedWidgets>) -> Result<(),()> {
    process(  &mut monitor
             , &mut rx
             , &mut tx).await;
    Ok(())
}

#[cfg(test)]
pub async fn behavior(mut monitor: SteadyMonitor, mut rx: SteadyRx<WidgetInventory>, mut tx: SteadyTx<ApprovedWidgets>) -> Result<(),()> {
    process( &mut monitor
              , &mut rx
              , &mut tx).await;
    Ok(())
}

// important function break out to ensure we have a point to test on
async fn process(mut monitor: &mut SteadyMonitor
                 , rx: &mut SteadyRx<WidgetInventory>
                 , tx: &mut SteadyTx<ApprovedWidgets>)  {
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
    use async_std::task;

    #[async_std::test]
    async fn test_process() {






    }

}