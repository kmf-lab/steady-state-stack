use std::ops::{ DerefMut};
use std::sync::Arc;
use async_std::sync::Mutex;
use bastion::run;
use log::*;
use crate::actor::data_approval::ApprovedWidgets;
use crate::steady::*;

struct InternalState {
    pub(crate) last_approval: Option<ApprovedWidgets>
}


#[cfg(not(test))]
pub async fn run(monitor: SteadyMonitor
                 , rx: Arc<Mutex<SteadyRx<ApprovedWidgets>>>) -> Result<(),()> {
    use std::time::Duration;

    let mut rx_guard = guard!(rx);
    let rx = ref_mut!(rx_guard);

    let mut monitor =  monitor.init_stats(&mut[rx], &mut[]);


    //here is alternative to batch, we send all the stats we have but only
    // every 100_000 received messages. This is helpful for very very high volumes.
    monitor.relay_stats_rx_set_custom_batch_limit(rx, 100_000);

    let mut state = InternalState { last_approval: None };
    loop {
        //single pass of work, in this high volume example we stay in iterate_once as long
        //as the input channel as more work to process.
        if iterate_once(&mut monitor, &mut state, rx).await {
            break Ok(());
        }
        //clear out any remaining stats but when we have no work do not spin tightly
        monitor.relay_stats_periodic(Duration::from_millis(40)).await;
    }
}



async fn iterate_once<const R: usize, const T: usize>(monitor: & mut LocalMonitor<R,T>
                                                      , state: &mut InternalState
                                                      , rx: &mut SteadyRx<ApprovedWidgets>) -> bool  {



    //example of high volume processing, we stay here until there is no more work BUT
    //we must also relay our telemetry data periodically
    while !rx.is_empty() {

            match monitor.take_async(rx).await {
                Ok(m) => {
                    state.last_approval = Some(m.to_owned());
                    trace!("received: {:?}", m.to_owned());
                },
                Err(msg) => {
                    state.last_approval = None;
                    error!("Unexpected error recv_async: {}",msg);
                }
            }

            //based on the channel capacity this will send batched updates so most calls do nothing.
            monitor.relay_stats_batch().await;

    }
    false

}


#[cfg(test)]
pub async fn run(monitor: SteadyMonitor
                 , rx: Arc<Mutex<SteadyRx<ApprovedWidgets>>>) -> Result<(),()> {
    //guards for the channels, NOTE: we could share one channel across actors.
    let mut rx_guard = rx.lock().await;
    //usable channels
    let rx = ref_mut!(rx_guard);

    let mut monitor = monitor.init_stats(&mut[rx], &mut[]);

    loop {
        relay_test(&mut monitor, rx).await;
        if false {
            break;
        }
    }
    Ok(())
}


#[cfg(test)]
async fn relay_test(monitor: &mut LocalMonitor<1, 0>, rx: &mut SteadyRx< ApprovedWidgets>) {
    use bastion::prelude::*;

    if let Some(ctx) = monitor.ctx() {
        MessageHandler::new(ctx.recv().await.unwrap())
            .on_question(|expected: ApprovedWidgets, answer_sender| {
                run!(async {
                let recevied = monitor.take_async(rx).await.unwrap();
                answer_sender.reply(if expected == recevied {"ok"} else {"err"}).unwrap();
               });
            });
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;

    #[test]
    async fn test_iterate_once() {

        crate::steady::tests::initialize_logger();

        let mut graph = SteadyGraph::new();
        let (tx, rx) = graph.new_channel(8,&[]);
        let mock_monitor = graph.new_test_monitor("consumer_monitor");

        let mut steady_tx_guard = guard!(tx);
        let mut steady_rx_guard = guard!(rx);

        let steady_tx = ref_mut!(steady_tx_guard);
        let steady_rx = ref_mut!(steady_rx_guard);


        let mut mock_monitor = mock_monitor.init_stats(&mut[], &mut[]);
        let mut state = InternalState { last_approval: None };

        let _ = mock_monitor.send_async(steady_tx, ApprovedWidgets {
            original_count: 1,
            approved_count: 2
        }).await;


        let exit= iterate_once(&mut mock_monitor, &mut state, steady_rx).await;

        assert_eq!(exit, false);
        let widgets = state.last_approval.unwrap();
        assert_eq!(widgets.original_count, 1);
        assert_eq!(widgets.approved_count, 2);

    }

}