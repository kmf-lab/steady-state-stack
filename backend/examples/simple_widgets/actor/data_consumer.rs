
use std::error::Error;


#[allow(unused_imports)]
use log::*;
use crate::actor::data_approval::ApprovedWidgets;
use steady_state::*;
use steady_state::monitor::LocalMonitor;
use crate::args::Args;

const BATCH_SIZE: usize = 2000;
#[derive(Clone, Debug, PartialEq, Copy)]
struct InternalState {
    pub(crate) last_approval: Option<ApprovedWidgets>,
    buffer: [ApprovedWidgets; BATCH_SIZE]
}


#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<ApprovedWidgets>) -> Result<(),Box<dyn Error>> {

    //let args:Option<&Args> = context.args(); //you can make the type explicit
    let args = context.args::<Args>(); //or you can turbo fish here to get your args


    let mut monitor = into_monitor!(context,[rx],[]);

    let mut rx = rx.lock().await;

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
    while monitor.is_running(&mut || rx.is_empty() && rx.is_closed() ) {

        wait_for_all!(monitor.wait_avail_units(&mut rx,1)).await;

        //single pass of work, in this high volume example we stay in iterate_once as long
        //as the input channel as more work to process.
        iterate_once(&mut monitor, &mut state, &mut rx).await;
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
               process_msg(state, Some(state.buffer[x].to_owned()));
            }

            //based on the channel capacity this will send batched updates so most calls do nothing.
            monitor.relay_stats_smartly().await;

    }


    false

}

#[inline]
fn process_msg(state: &mut InternalState, msg: Option<ApprovedWidgets>) {
    match msg {
        Some(m) => {
            state.last_approval = Some(m);
            //trace!("received: {:?}", m.to_owned());
        },
        None => {
            state.last_approval = None;
            //error!("Unexpected error recv_async: {}",_msg);
        }
    }
}


#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<ApprovedWidgets>) -> Result<(),Box<dyn Error>> {
    let mut monitor = into_monitor!(context,[rx],[]);

    let mut rx = rx.lock().await;

    while monitor.is_running(&mut || rx.is_empty() && rx.is_closed()) {

        wait_for_all!(monitor.wait_avail_units(&mut rx,1)).await;

        relay_test(&mut monitor, &mut rx).await;

    }
    Ok(())
}

// fn downcast_trait_object<T: 'static>(obj: &(dyn BackPlaneMessage + 'static)) -> Option<&T> {
//     obj.as_any().downcast_ref::<T>()
// }

/*
fn cast_to_aw(obj: &dyn BackPlaneMessage) -> Option<&ApprovedWidgets> {
    BackPlaneMessage::downcast_ref::<ApprovedWidgets>(obj)
   // obj.downcast_ref::<ApprovedWidgets>()
}*/

#[cfg(test)]
async fn relay_test(monitor: &mut LocalMonitor<1, 0>, rx: &mut Rx< ApprovedWidgets>) {

    if let Some(simulator) = monitor.sidechannel_responder() {
        simulator.respond_with(|expected| {

            rx.block_until_not_empty(std::time::Duration::from_secs(20));
            match monitor.try_take(rx) {
                Some(measured) => {
                    let expected: &ApprovedWidgets = expected.downcast_ref::<ApprovedWidgets>().expect("error casting");

                        let result = expected.cmp(&measured);
                        if result.is_eq() {
                            Box::new("".to_string())
                        } else {
                            Box::new(format!("no match {:?} {:?} {:?}"
                                             , expected
                                             , result
                                             , measured).to_string())
                        }

                },
                None => Box::new("no data".to_string()),
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

        let steady_tx = &mut *steady_tx_guard;
        let steady_rx = &mut *steady_rx_guard;


        let mut mock_monitor = mock_monitor.into_monitor([], []);
        let mut state = InternalState {
            last_approval: None,
            buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]

        };

        let _ = mock_monitor.send_async(steady_tx, ApprovedWidgets {
            original_count: 1,
            approved_count: 2,


        },SendSaturation::Warn).await;


        let exit= iterate_once(&mut mock_monitor, &mut state, steady_rx).await;

        assert_eq!(exit, false);
        let widgets = state.last_approval.unwrap();
        assert_eq!(widgets.original_count, 1);
        assert_eq!(widgets.approved_count, 2);

    }

}