use std::time::Duration;
use steady_state::*;
use log::*;
use steady_state::monitor::LocalMonitor;
use steady_state::{Rx, SteadyRx};
use steady_state::{SteadyTx, Tx};

#[derive(Clone, Debug, PartialEq)]
pub struct SomeExampleRecord {
}

#[derive(Clone, Debug)]
struct SomeLocalState {
}

//example code is not called so we let the compiler know
#[allow(dead_code)]
#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , tx: SteadyTx<SomeExampleRecord>
                 , rx: SteadyRx<SomeExampleRecord>) -> Result<(),()> {

    let mut monitor = into_monitor!(context, [rx], [tx]);
    let mut state = SomeLocalState{};

    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    while monitor.is_running(&mut || rx.is_empty() && rx.is_closed() && tx.mark_closed()) {

        //single pass of work, do not loop in here
        iterate_once( &mut monitor
                        , &mut state
                        , &mut tx
                        , &mut rx).await;
        //this is an example of an telemetry running periodically
        //we send telemetry and wait for the next time we are to run here
        monitor.relay_stats_periodic(Duration::from_millis(40)).await;
    }
    Ok(())
}

//example code is not called so we let the compiler know
#[allow(dead_code)]
#[cfg(test)]
pub async fn run(context: SteadyContext
                 , tx: SteadyTx<SomeExampleRecord>
                 , rx: SteadyRx<SomeExampleRecord>) -> Result<(),()> {

    let mut monitor = into_monitor!(context,[rx],[tx]);

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
            Some(m) => {
                let _ = monitor.send_async(tx, m,SendSaturation::default()).await;
            },
            None => {
                error!("Unexpected error recv_async, probably during shutdown");
            }
        }
        monitor.relay_stats_smartly();
    }
    false
}

#[cfg(test)]
mod tests {

    use steady_state::*;



    #[async_std::test]
    async fn test_process_function() {

        util::logger::initialize();

        let _graph = Graph::new("");




    }

}
