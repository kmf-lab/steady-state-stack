use std::time::Duration;

use log::*;
use crate::steady::*;

#[derive(Clone, Debug, PartialEq)]
pub struct SomeExampleRecord {
}

#[derive(Clone, Debug)]
struct SomeLocalState {
}

//example code is not called so we let the compiler know
#[allow(dead_code)]
#[cfg(not(test))]
pub async fn run(mut monitor: SteadyMonitor
                 , tx: SteadyTx<SomeExampleRecord>
                 , rx: SteadyRx<SomeExampleRecord>) -> Result<(),()> {
    let mut state = SomeLocalState{};
    let mut monitor = monitor.init_stats(&[&rx], &[&tx]);

    loop {
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                        , &mut state
                        , &tx
                        , &rx).await {
            break Ok(());
        }
        //this is an example of an actor running periodically
        //we send telemetry and wait for the next time we are to run here
        monitor.relay_stats_periodic(Duration::from_millis(40)).await;
    }

}

//example code is not called so we let the compiler know
#[allow(dead_code)]
#[cfg(test)]
pub async fn run(mut monitor: SteadyMonitor
                  , tx: SteadyTx<SomeExampleRecord>
                  , rx: SteadyRx<SomeExampleRecord>) -> Result<(),()> {
    let mut state = SomeLocalState{};
    let mut monitor = monitor.init_stats(&[&rx], &[&tx]);

    loop {
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                        , &mut state
                      , &tx
                      , &rx).await {
            break Ok(());
        }
        //when the outgoing pipe is full or the input is empty we do not want to spin
        //so this will send the telemetry at a lower rate and await the next time to run
        monitor.relay_stats_periodic(Duration::from_millis(40)).await;
    }
}

async fn iterate_once(monitor: &mut LocalMonitor<1, 1>
                        , _state: &mut SomeLocalState
                      , tx: &SteadyTx<SomeExampleRecord>
                      , rx: &SteadyRx<SomeExampleRecord>
                ) -> bool
{
    //continue to process until we have no more work or there is no more room to send
    while rx.has_message() && tx.has_room() {
        match monitor.rx(rx).await {
            Ok(m) => {
                let _ = monitor.tx(tx, m).await;
            },
            Err(e) => {
                error!("Unexpected error recv_async: {}",e);
            }
        }

        //we could use monitor.relay_status_batch here but this is high volume example so
        //this demonstrates processing as much a possible and only sending telemetry based
        //on our own batch size after n messages in or out.
        monitor.relay_stats_rx_custom(rx, 200_000).await;
        monitor.relay_stats_tx_custom(tx, 300_000).await;

    }
    false
}

#[cfg(test)]
mod tests {
    use crate::actor::example_empty_actor::{iterate_once, SomeExampleRecord, SomeLocalState};
    use crate::steady::{SteadyGraph, SteadyTx};

    #[async_std::test]
    async fn test_process_function() {

        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_in, rx_in): (SteadyTx<SomeExampleRecord>, _) = graph.new_channel(8,&[]);
        let (tx_out, rx_out): (SteadyTx<SomeExampleRecord>, _) = graph.new_channel(8,&[]);

        let mut mock_monitor = graph.new_test_monitor("example_test").await;

        let mut mock_monitor = mock_monitor.init_stats(&[&rx_in], &[&tx_out]);
        let mut state = SomeLocalState{};

        let _ = mock_monitor.tx(&tx_in, SomeExampleRecord{}).await;
        let result = iterate_once(&mut mock_monitor, &mut state, &tx_out, &rx_in).await;
        assert_eq!(false, result);
        assert_eq!(true, rx_out.has_message());
        let msg = mock_monitor.rx(&rx_out).await;
        assert_eq!(SomeExampleRecord{}, msg.unwrap());
    }

}