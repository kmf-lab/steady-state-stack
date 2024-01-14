use std::sync::Arc;
use std::time::Duration;
use async_std::sync::Mutex;

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
pub async fn run(monitor: SteadyMonitor
                 , tx: Arc<Mutex<SteadyTx<SomeExampleRecord>>>
                 , rx: Arc<Mutex<SteadyRx<SomeExampleRecord>>>) -> Result<(),()> {

    let mut tx_guard = guard!(tx);
    let mut rx_guard = guard!(rx);
    let tx = ref_mut!(tx_guard);
    let rx = ref_mut!(rx_guard);

    let mut monitor = monitor.init_stats(&mut [rx], &mut [tx]);
    let mut state = SomeLocalState{};

    loop {
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                        , &mut state
                        , tx
                        , rx).await {
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
pub async fn run(monitor: SteadyMonitor
                 , tx: Arc<Mutex<SteadyTx<SomeExampleRecord>>>
                 , rx: Arc<Mutex<SteadyRx<SomeExampleRecord>>>) -> Result<(),()> {

    let mut tx_guard = guard!(tx);
    let mut rx_guard = guard!(rx);
    let tx = ref_mut!(tx_guard);
    let rx = ref_mut!(rx_guard);

    let mut monitor = monitor.init_stats(&mut [rx], &mut [tx]);
    let mut state = SomeLocalState{};

    //this is high volume example so
    //this demonstrates processing as much a possible and only sending telemetry based
    //on our own batch size after n messages in or out.
    monitor.relay_stats_rx_set_custom_batch_limit(rx, 200_000);
    monitor.relay_stats_tx_set_custom_batch_limit(tx, 300_000);


    loop {
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                        , &mut state
                      , tx
                      , rx).await {
            break Ok(());
        }
        //when the outgoing pipe is full or the input is empty we do not want to spin
        //so this will send the telemetry at a lower rate and await the next time to run
        monitor.relay_stats_periodic(Duration::from_millis(40)).await;
    }
}

async fn iterate_once(monitor: &mut LocalMonitor<1, 1>
                        , _state: &mut SomeLocalState
                      , tx: &mut SteadyTx<SomeExampleRecord>
                      , rx: &mut SteadyRx<SomeExampleRecord>
                ) -> bool
{

    //continue to process until we have no more work or there is no more room to send
    while (!rx.is_empty()) && !tx.is_full() {
        match monitor.take_async(rx).await {
            Ok(m) => {
                let _ = monitor.send_async(tx, m).await;
            },
            Err(msg) => {
                error!("Unexpected error recv_async {}", msg);
            }
        }
        monitor.relay_stats_batch().await;
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::actor::example_empty_actor::{iterate_once, SomeExampleRecord, SomeLocalState};
    use crate::steady::{SteadyGraph};

    #[async_std::test]
    async fn test_process_function() {

        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx_in, rx_in) = graph.new_channel(8,&[]);
        let (tx_out, rx_out) = graph.new_channel(8,&[]);

        let mut tx_in_guard = guard!(tx_in);
        let mut rx_in_guard = guard!(rx_in);
        let mut tx_out_guard = guard!(tx_out);
        let mut rx_out_guard = guard!(rx_out);

        let tx_in = ref_mut!(tx_in_guard);
        let rx_in = ref_mut!(rx_in_guard);
        let tx_out = ref_mut!(tx_out_guard);
        let rx_out = ref_mut!(rx_out_guard);


        let mock_monitor = graph.new_test_monitor("example_test");

        let mut mock_monitor = mock_monitor.init_stats(&mut[rx_in], &mut[tx_out]);
        let mut state = SomeLocalState{};

        let _ = mock_monitor.send_async(tx_in, SomeExampleRecord{}).await;
        let result = iterate_once(&mut mock_monitor, &mut state, tx_out, rx_in).await;
        assert_eq!(false, result);
        assert_eq!(false, rx_out.is_empty());
        let msg = mock_monitor.take_async(rx_out).await;
        assert_eq!(SomeExampleRecord{}, msg.unwrap());
    }

}
