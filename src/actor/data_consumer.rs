use log::*;
use crate::actor::data_approval::ApprovedWidgets;
use crate::steady::*;

struct InternalState {
    pub(crate) last_approval: Option<ApprovedWidgets>
}

async fn iterate_once(monitor: &mut SteadyMonitor
                 , state: &mut InternalState
                 , rx_approved_widgets: &SteadyRx<ApprovedWidgets>) -> bool  {

    //example of high volume processing, we stay here until there is no more work BUT
    //we must also relay our telemetry data periodically
    while rx_approved_widgets.has_message() {
        match monitor.rx(rx_approved_widgets).await {
            Ok(m) => {
                state.last_approval = Some(m.to_owned());
                info!("received: {:?}", m.to_owned());
            },
            Err(e) => {
                state.last_approval = None;
                error!("Unexpected error recv_async: {}",e);
            }
        }
        //based on the channel capacity this will send batched updates so most calls do nothing.
        monitor.relay_stats_batch().await;
        //here is alternative to batch, we send all the stats we have but only
        // every 100_000 received messages. This is helpful for very very high volumes.
        monitor.relay_stats_rx_custom(rx_approved_widgets, 100_000).await;
    }
    false

}

#[cfg(not(test))]
pub async fn run(mut monitor: SteadyMonitor, mut rx_approved_widgets: SteadyRx<ApprovedWidgets>) -> Result<(),()> {
    use std::time::Duration;

    let mut state = InternalState { last_approval: None };
    loop {
        //single pass of work, in this high volume example we stay in iterate_once as long
        //as the input channel as more work to process.
        if iterate_once(&mut monitor, &mut state, &mut rx_approved_widgets).await {
            break Ok(());
        }
        //clear out any remaining stats but when we have no work do not spin tightly
        monitor.relay_stats_periodic(Duration::from_millis(40)).await;
    }
}

#[cfg(test)]
pub async fn run(mut monitor: SteadyMonitor, rx_approved_widgets: SteadyRx<ApprovedWidgets>) -> Result<(),()> {
    loop {
        relay_test(&mut monitor, &rx_approved_widgets).await;
        if false {
            break;
        }
    }
    Ok(())
}
#[cfg(test)]
async fn relay_test(monitor: &mut SteadyMonitor, rx: &SteadyRx<ApprovedWidgets>) {
    use bastion::prelude::*;


    let ctx = {
        monitor.ctx.as_ref().unwrap()
    };
    MessageHandler::new(ctx.recv().await.unwrap())
        .on_question( |expected: ApprovedWidgets, answer_sender| {
            run!(async {
                let recevied = monitor.rx(&rx).await.unwrap();
                answer_sender.reply(if expected == recevied {"ok"} else {"err"}).unwrap();
               });
        });
}



#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;

    #[test]
    async fn test_iterate_once() {

        crate::steady::tests::initialize_logger();


        let mut graph = SteadyGraph::new();
        let (tx, rx): (SteadyTx<ApprovedWidgets>, _) = graph.new_channel(8);
        let mut mock_monitor = graph.new_test_monitor("consumer_monitor").await;

        mock_monitor.tx(&tx, ApprovedWidgets {
            original_count: 1,
            approved_count: 2
        }).await;

        let mut state = InternalState { last_approval: None };

        let exit= iterate_once(&mut mock_monitor, &mut state, &rx).await;

        assert_eq!(exit, false);
        let widgets = state.last_approval.unwrap();
        assert_eq!(widgets.original_count, 1);
        assert_eq!(widgets.approved_count, 2);

    }

}