use std::time::Duration;

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
        if iterate_once( &mut monitor
                        , &mut tx
                        , &mut rx).await {
            break Ok(());
        }
        monitor.relay_stats_periodic(Duration::from_millis(3000)).await;
    }
}

#[cfg(test)]
pub async fn _behavior(mut monitor: SteadyMonitor
                      , mut tx: SteadyTx<SomeExampleRecord>
                      , mut rx: SteadyRx<SomeExampleRecord>) -> Result<(),()> {
    loop {
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                      , &mut tx
                      , &mut rx).await {
            break Ok(());
        }
        monitor.relay_stats_periodic(Duration::from_millis(3000)).await;
    }
}

async fn iterate_once(monitor: &mut SteadyMonitor
                      , tx: &SteadyTx<SomeExampleRecord>
                      , rx: &SteadyRx<SomeExampleRecord>
                ) -> bool
{

    if rx.has_message() && tx.has_room() {
        match monitor.rx(rx).await {
            Ok(m) => {
                monitor.tx(tx, m).await;
            },
            Err(e) => {
                error!("Unexpected error recv_async: {}",e);
            }
        }

    }
    false
}

#[cfg(test)]
mod tests {
    use crate::actor::example_empty_actor::{iterate_once, SomeExampleRecord};
    use crate::steady::{SteadyGraph, SteadyTx};

    #[async_std::test]
    async fn test_process_function() {

        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx, rx): (SteadyTx<SomeExampleRecord>, _) = graph.new_channel(8);
        let mut monitor = graph.new_monitor().await.wrap("test",None);

        let result = iterate_once(&mut monitor, &tx, &rx).await;


    }

}