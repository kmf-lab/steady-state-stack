
use std::error::Error;


#[allow(unused_imports)]
use log::*;
use crate::actor::data_approval::ApprovedWidgets;
use steady_state::*;
use steady_state::monitor::LocalMonitor;
use steady_state::{Rx, SteadyRx};
use crate::args::Args;

const BATCH_SIZE: usize = 1000;
#[derive(Clone, Debug, PartialEq, Copy)]
struct InternalState {
    pub(crate) last_approval: Option<ApprovedWidgets>,
    buffer: [ApprovedWidgets; BATCH_SIZE]
}


#[cfg(not(test))]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<ApprovedWidgets>) -> Result<(),Box<dyn Error>> {
    let state = InternalState {
        last_approval: None,
        buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
    };
    internal_behavior(context, rx, state).await
}

pub(crate) async fn internal_behavior(context: SteadyContext, rx: SteadyRx<ApprovedWidgets>, mut state: InternalState) -> Result<(), Box<dyn Error>> {
    //let args:Option<&Args> = context.args(); //you can make the type explicit
    let _args = context.args::<Args>(); //or you can turbo fish here to get your args
    //trace!("running {:?} {:?}",context.id(),context.name());


    let mut monitor = into_monitor!(context,[rx],[]);

    let mut rx = rx.lock().await;

    //predicate which affirms or denies the shutdown request
    while monitor.is_running(&mut || rx.is_closed_and_empty()) {
        let _clean = wait_for_all!(monitor.wait_avail_units(&mut rx,1)).await;

        //example of high volume processing, we stay here until there is no more work BUT
        //we must also relay our telemetry data periodically
        while !rx.is_empty() {
            let mut buf = state.buffer;;
            let count = monitor.take_slice(&mut rx, &mut buf);
            for x in 0..count {
                state.last_approval = Some(buf[x].to_owned());
            }
            //based on the channel capacity this will send batched updates so most calls do nothing.
            monitor.relay_stats_smartly();

        }
    }
    Ok(())
}





#[cfg(test)]
pub async fn run(context: SteadyContext
                 , rx: SteadyRx<ApprovedWidgets>) -> Result<(),Box<dyn Error>> {
    let mut monitor = into_monitor!(context,[rx],[]);
    let mut rx = rx.lock().await;

    if let Some(simulator) = monitor.sidechannel_responder() {
        while monitor.is_running(&mut || rx.is_closed_and_empty()) {

        let _clean = wait_for_all!(monitor.wait_avail_units(&mut rx,1)).await;
            simulator.respond_with(|expected| {
                match monitor.try_take(&mut rx) {
                    Some(measured) => {
                        let expected: &ApprovedWidgets = expected.downcast_ref::<ApprovedWidgets>().expect("error casting");

                        if expected.cmp(&measured).is_eq() {
                            Box::new("ok".to_string())
                        } else {
                            let failure = format!("no match {:?} {:?}"
                                                  , expected
                                                  , measured).to_string();
                            error!("failure: {}", failure);
                            Box::new(failure)
                        }

                    },
                    None => Box::new("no data".to_string()),
                }

            }).await;
        }

    }

    Ok(())
}





#[cfg(test)]
mod tests {
    use std::time::Duration;
    use super::*;
    use async_std::test;
    use steady_state::Graph;


    #[test]
    pub(crate) async fn test_simple_process() {
        //1. build test graph, the input and output channels and our actor
        let mut graph = Graph::new_test(());

        let (approved_widget_tx_out,approved_widget_rx_out) = graph.channel_builder()
            .with_capacity(BATCH_SIZE).build();

        let state = InternalState {
            last_approval: None,
            buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
        };

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn(move |context| internal_behavior(context, approved_widget_rx_out.clone(), state));

        // //2. add test data to the input channels
        let test_data: Vec<ApprovedWidgets> = (0..BATCH_SIZE).map(|i| ApprovedWidgets { original_count: 0, approved_count: i as u64 }).collect();
        approved_widget_tx_out.testing_send(test_data, Duration::from_millis(30), true).await;

        // //3. run graph until the actor detects the input is closed
         graph.start_as_data_driven(Duration::from_secs(240));

        //4. assert expected results
        // TODO: not sure how to make this work.
        //  println!("last approval: {:?}", &state.last_approval);
        //  assert_eq!(approved_widget_rx_out.testing_avail_units().await, BATCH_SIZE);


    }



}