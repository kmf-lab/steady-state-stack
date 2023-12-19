use std::time::Duration;
use flume::{Receiver, Sender};
use futures::select;
use futures_timer::Delay;
use crate::steady::*;
use futures::FutureExt;

pub struct SomeExampleRecord {

}

#[cfg(not(test))]
pub async fn behavior(mut monitor: SteadyMonitor, _tx: Sender<SomeExampleRecord>, _rx: Receiver<SomeExampleRecord>) -> Result<(),()> {
    loop {
        select! {
            _ = monitor.relay_stats().await => {},
            _ = process( &mut monitor
                       // , &mut tx // put your args here
                         ).fuse() => {}
        }
    }
    Ok(())
}

#[cfg(test)]
pub async fn behavior(mut monitor: SteadyMonitor, _tx: Sender<SomeExampleRecord>, _rx: Receiver<SomeExampleRecord>) -> Result<(),()> {

    todo!(); // put code here for testing the full graph, this injects tx and reads rx back to the tester

    Ok(())
}

async fn process(monitor: &mut SteadyMonitor
                // , tx_widget: &mut SteadyTx<WidgetInventory>
                )
{
    loop {
        Delay::new(Duration::from_secs(3)).await
    }
}

#[cfg(test)]
mod tests {

    #[async_std::test]
    async fn test_process_function() {

    }

}