
use std::error::Error;


#[allow(unused_imports)]
use log::*;
use crate::actor::data_approval::ApprovedWidgets;
use steady_state::*;
use steady_state::monitor::LocalMonitor;
use steady_state::{Rx, SteadyRx};
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
    _internal_behavior(context, rx).await
}

async fn _internal_behavior(context: SteadyContext, rx: SteadyRx<ApprovedWidgets>) -> Result<(), Box<dyn Error>> {
    //let args:Option<&Args> = context.args(); //you can make the type explicit
    let _args = context.args::<Args>(); //or you can turbo fish here to get your args
    //trace!("running {:?} {:?}",context.id(),context.name());


    let mut monitor = into_monitor!(context,[rx],[]);

    let mut rx = rx.lock().await;

    let mut state = InternalState {
        last_approval: None,
        buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
    };


    //predicate which affirms or denies the shutdown request
    while monitor.is_running(&mut || rx.is_closed_and_empty()) {
        let _clean = wait_for_all!(monitor.wait_avail_units(&mut rx,1)).await;
        //single pass of work, in this high volume example we stay in iterate_once as long
        //as the input channel as more work to process.
        //wait for new work, we could also use a timer here to send telemetry periodically
        if rx.is_empty() {
            let msg = monitor.take_async(&mut rx).await;
            process_msg(&mut state, msg);
        }
        //example of high volume processing, we stay here until there is no more work BUT
        //we must also relay our telemetry data periodically
        while !rx.is_empty() {
            let mut buf = state.buffer;;
            let count = monitor.take_slice(&mut rx, &mut buf);
            for x in 0..count {
                process_msg(&mut state, Some(buf[x].to_owned()));
            }

            //based on the channel capacity this will send batched updates so most calls do nothing.
            monitor.relay_stats_smartly();

        }
    }
    Ok(())
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
    while monitor.is_running(&mut || rx.is_closed_and_empty()) {
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

        let block_fail_fast = false;
        let mut graph = Graph::internal_new((), block_fail_fast, false);
        let (tx, rx) = graph.channel_builder().with_capacity(8).build();
        let tx = tx.clone();
        let rx = rx.clone();
        let mock_context = graph.new_test_monitor("mock");

        let mut mock_monitor = into_monitor!(mock_context, [rx], [tx]);

        let mut tx = tx.lock().await;
        let mut rx = rx.lock().await;

        let mut state = InternalState {
            last_approval: None,
            buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
        };

        let _ = mock_monitor.send_async(&mut *tx, ApprovedWidgets {
            original_count: 1,
            approved_count: 2,
        },SendSaturation::Warn).await;

        //wait for new work, we could also use a timer here to send telemetry periodically
        if rx.is_empty() {
            let msg = mock_monitor.take_async(&mut rx).await;
            process_msg(&mut state, msg);
        }
        //example of high volume processing, we stay here until there is no more work BUT
        //we must also relay our telemetry data periodically
        while !rx.is_empty() {
            let mut buf = state.buffer;
            let count = mock_monitor.take_slice(&mut rx, &mut buf);
            for x in 0..count {
                process_msg(&mut state, Some(buf[x].to_owned()));
            }

            //based on the channel capacity this will send batched updates so most calls do nothing.
            mock_monitor.relay_stats_smartly();

        }

        let widgets = state.last_approval.unwrap();
        assert_eq!(widgets.original_count, 1);
        assert_eq!(widgets.approved_count, 2);

    }

}