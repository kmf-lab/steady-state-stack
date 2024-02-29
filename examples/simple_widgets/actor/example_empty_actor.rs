use std::ops::DerefMut;
use std::time::Duration;
use steady_state::*;
use log::*;
use steady_state::monitor::LocalMonitor;

#[derive(Clone, Debug, PartialEq)]
pub struct SomeExampleRecord {
}

#[derive(Clone, Debug)]
struct SomeLocalState {
}

//example code is not called so we let the compiler know
#[allow(dead_code)]
#[cfg(not(test))]
pub async fn run(monitor: SteadyContext
                 , tx: SteadyTx<SomeExampleRecord>
                 , rx: SteadyRx<SomeExampleRecord>) -> Result<(),()> {

    let mut monitor = monitor.into_monitor([&rx], [&tx]);
    let mut state = SomeLocalState{};

    let mut rx_guard = rx.lock().await;
    let mut tx_guard = tx.lock().await;
    let rx = rx_guard.deref_mut();
    let tx = tx_guard.deref_mut();

    loop {
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                        , &mut state
                        , tx
                        , rx).await {
            break Ok(());
        }
        //this is an example of an telemetry running periodically
        //we send telemetry and wait for the next time we are to run here
        monitor.relay_stats_periodic(Duration::from_millis(40)).await;
    }

}

//example code is not called so we let the compiler know
#[allow(dead_code)]
#[cfg(test)]
pub async fn run(monitor: SteadyContext
                 , tx: SteadyTx<SomeExampleRecord>
                 , rx: SteadyRx<SomeExampleRecord>) -> Result<(),()> {

    let mut monitor = monitor.into_monitor([&rx], [&tx]);

    let mut rx_guard = rx.lock().await;
    let mut tx_guard = tx.lock().await;
    let rx = rx_guard.deref_mut();
    let tx = tx_guard.deref_mut();

    let mut state = SomeLocalState{};



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
                      , tx: &mut Tx<SomeExampleRecord>
                      , rx: &mut Rx<SomeExampleRecord>
                ) -> bool
{

    //continue to process until we have no more work or there is no more room to send
    while (!rx.is_empty()) && !tx.is_full() {
        match monitor.take_async(rx).await {
            Ok(m) => {
                let _ = monitor.send_async(tx, m,false).await;
            },
            Err(msg) => {
                error!("Unexpected error recv_async {}", msg);
            }
        }
        monitor.relay_stats_smartly().await;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::ops::DerefMut;
    use steady_state::*;
    use crate::actor::example_empty_actor::{iterate_once, SomeExampleRecord, SomeLocalState};


    #[async_std::test]
    async fn test_process_function() {

        util::logger::initialize();

        let mut graph = Graph::new("");
        let (tx_in, rx_in) = graph.channel_builder().with_capacity(8).build();
        let (tx_out, rx_out) = graph.channel_builder().with_capacity(8).build();

        let mock_monitor = graph.new_test_monitor("example_test");
        let mut mock_monitor = mock_monitor.into_monitor([&rx_in], [&tx_out]);

        let mut tx_in_guard = tx_in.lock().await;
        let mut rx_in_guard = rx_in.lock().await;
        let mut tx_out_guard = tx_out.lock().await;
        let mut rx_out_guard = rx_out.lock().await;

        let tx_in = tx_in_guard.deref_mut();
        let rx_in = rx_in_guard.deref_mut();
        let tx_out = tx_out_guard.deref_mut();
        let rx_out = rx_out_guard.deref_mut();


        let mut state = SomeLocalState{};

        let _ = mock_monitor.send_async(tx_in, SomeExampleRecord{}, false).await;
        let result = iterate_once(&mut mock_monitor, &mut state, tx_out, rx_in).await;
        assert_eq!(false, result);
        assert_eq!(false, rx_out.is_empty());
        let msg = mock_monitor.take_async(rx_out).await;
        assert_eq!(SomeExampleRecord{}, msg.unwrap());
    }

}
