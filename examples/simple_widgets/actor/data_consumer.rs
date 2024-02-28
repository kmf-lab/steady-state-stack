use std::ops::DerefMut;
use std::time::Duration;

#[allow(unused_imports)]
use log::*;
use crate::actor::data_approval::ApprovedWidgets;
use steady_state::*;

const BATCH_SIZE: usize = 2000;
#[derive(Clone, Debug, PartialEq, Copy)]
struct InternalState {
    pub(crate) last_approval: Option<ApprovedWidgets>,
    buffer: [ApprovedWidgets; BATCH_SIZE]
}


#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<ApprovedWidgets>) -> Result<(),()> {




    //let args:Option<&Args> = context.args(); //you can make the type explicit
    //let args = context.args::<Args>(); //or you can turbo fish here to get your args


    let mut monitor =  context.into_monitor([&rx], []);

    let mut rx_guard = rx.lock().await;
    let rx = rx_guard.deref_mut();

    let mut state = InternalState {
        last_approval: None,
        buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
    };

    //do avoid blocking code but if you must...
    blocking!({
        trace!("this is an example of blocking code");
        //you could do some short blocking work as needed but
        //while here the telemetry will not be sent and
        //the actor will not check for shutdown
        //NOTE: stay tuned for a better feature on the way.
    });


    //predicate which affirms or denies the shutdown request
    while monitor.is_running(&mut || rx.is_closed()) {
        //single pass of work, in this high volume example we stay in iterate_once as long
        //as the input channel as more work to process.
        if iterate_once(&mut monitor, &mut state, rx).await {
            return Ok(());
        }
    }
    Ok(())
}


async fn iterate_once<const R: usize, const T: usize>(monitor: & mut LocalMonitor<R,T>
                                                      , state: &mut InternalState
                                                      , rx: &mut Rx<ApprovedWidgets>) -> bool  {

    //wait for new work, we could also use a timer here to send telemetry periodically
    if rx.is_empty() {
        let msg = monitor.take_async(rx).await;
        process_msg(state, msg);
    }
    //example of high volume processing, we stay here until there is no more work BUT
    //we must also relay our telemetry data periodically
    while !rx.is_empty() {
            let count = monitor.take_slice(rx, &mut state.buffer);
            for x in 0..count {
               process_msg(state, Ok(state.buffer[x].to_owned()));
            }

            //based on the channel capacity this will send batched updates so most calls do nothing.
            monitor.relay_stats_smartly().await;

    }


    false

}

#[inline]
fn process_msg(state: &mut InternalState, msg: Result<ApprovedWidgets, String>) {
    match msg {
        Ok(m) => {
            state.last_approval = Some(m);
            //trace!("received: {:?}", m.to_owned());
        },
        Err(_msg) => {
            state.last_approval = None;
            //error!("Unexpected error recv_async: {}",_msg);
        }
    }
}


#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<ApprovedWidgets>) -> Result<(),()> {
    let mut monitor = context.into_monitor([&rx], []);

    //guards for the channels, NOTE: we could share one channel across actors.
    let mut rx_guard = rx.lock().await;
    let rx = rx_guard.deref_mut();


    loop {
        relay_test(&mut monitor, rx).await;
        if false {
            break;
        }
    }
    Ok(())
}


#[cfg(test)]
async fn relay_test(monitor: &mut LocalMonitor<1, 0>, rx: &mut Rx< ApprovedWidgets>) {

    if let Some(simulator) = monitor.edge_simulator() {
        simulator.respond_to_request(|expected: ApprovedWidgets| {

            rx.block_until_not_empty(Duration::from_secs(20));
            match monitor.try_take(rx) {
                Some(measured) => {
                    let result = expected.cmp(&measured);
                    if result.is_eq() {
                        GraphTestResult::Ok(())
                    } else {
                        GraphTestResult::Err(format!("no match {:?} {:?} {:?}"
                                             ,expected
                                             ,result
                                             ,measured))
                    }
                },
                None => GraphTestResult::Err("no data".to_string()),
            }

        }).await;
    }

}



#[cfg(test)]
mod tests {
    use super::*;
    use async_std::test;
    use steady_state::Graph;

    #[test]
    async fn test_iterate_once() {

        util::logger::initialize();

        let mut graph = Graph::new("");
        let (tx, rx) = graph.channel_builder().with_capacity(8).build();
        let mock_monitor = graph.new_test_monitor("consumer_monitor");

        let mut steady_tx_guard = tx.lock().await;
        let mut steady_rx_guard = rx.lock().await;

        let steady_tx = steady_tx_guard.deref_mut();
        let steady_rx = steady_rx_guard.deref_mut();


        let mut mock_monitor = mock_monitor.into_monitor([], []);
        let mut state = InternalState {
            last_approval: None,
            buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]

        };

        let _ = mock_monitor.send_async(steady_tx, ApprovedWidgets {
            original_count: 1,
            approved_count: 2,


        }).await;


        let exit= iterate_once(&mut mock_monitor, &mut state, steady_rx).await;

        assert_eq!(exit, false);
        let widgets = state.last_approval.unwrap();
        assert_eq!(widgets.original_count, 1);
        assert_eq!(widgets.approved_count, 2);

    }

}