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

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    loop {
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                        , &mut state
                        , &mut tx
                        , &mut rx).await {
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

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    let mut state = SomeLocalState{};

    loop {
        //single pass of work, do not loop in here
        if iterate_once( &mut monitor
                        , &mut state
                      , &mut tx
                      , &mut rx).await {
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




    }

}
