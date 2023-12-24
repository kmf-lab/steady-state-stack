use std::time::Duration;
use futures::future;

use log::*;
use crate::steady::*;

#[derive(Clone, Debug)]
pub struct SomeExampleRecord {

}

#[cfg(not(test))]
pub async fn behavior(mut monitor: SteadyMonitor
                      , mut tx: SteadyTx<SomeExampleRecord>
                      , mut rx: SteadyRx<SomeExampleRecord>) -> Result<(),()> {

    loop {
        //single pass of work, do not loop in here
        iterate_once( &mut monitor
                        , &mut tx
                        , &mut rx).await;
        monitor.relay_stats_periodic(Duration::from_millis(3000)).await;
    }
    future::pending().await
}

#[cfg(test)]
pub async fn _behavior(mut monitor: SteadyMonitor
                      , mut tx: SteadyTx<SomeExampleRecord>
                      , mut rx: SteadyRx<SomeExampleRecord>) -> Result<(),()> {
    loop {
        //single pass of work, do not loop in here
        iterate_once( &mut monitor
                      , &mut tx
                      , &mut rx).await;
        monitor.relay_stats_periodic(Duration::from_millis(3000)).await;
    }
    unreachable!("This code should never be reached otherwise Ok(()) should have been returned")
}

async fn iterate_once(mut monitor: &mut SteadyMonitor
                      , tx: &SteadyTx<SomeExampleRecord>
                      , rx: &SteadyRx<SomeExampleRecord>
                )
{

    if rx.has_message() && tx.has_room() {
        match rx.rx(&mut monitor).await {
            Ok(m) => {
                tx.tx(&mut monitor, m).await;
            },
            Err(e) => {
                error!("Unexpected error recv_async: {}",e);
            }
        }

    }

}

#[cfg(test)]
mod tests {

    #[async_std::test]
    async fn test_process_function() {

        // TODO: test iterate_once

    }

}