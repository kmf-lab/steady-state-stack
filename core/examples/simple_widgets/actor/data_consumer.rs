use std::error::Error;
use std::sync::Arc;
use futures_util::lock::Mutex;
#[allow(unused_imports)]
use log::*;
use crate::actor::data_approval::ApprovedWidgets;
use steady_state::*;
use crate::args::Args;

const BATCH_SIZE: usize = 1000;
#[derive(Clone, Debug, PartialEq, Copy)]
pub(crate) struct InternalState {
    pub(crate) last_approval: Option<ApprovedWidgets>,
    buffer: [ApprovedWidgets; BATCH_SIZE]
}

impl InternalState {
    pub fn new() -> Self {
        InternalState {
            last_approval: None,
            buffer: [ApprovedWidgets { approved_count: 0, original_count: 0 }; BATCH_SIZE]
        }
    }
}

pub async fn run(context: SteadyActorShadow
                 , rx: SteadyRx<ApprovedWidgets>
                 , state: Arc<Mutex<InternalState>>) -> Result<(),Box<dyn Error>> {
    let actor = context.into_spotlight([&rx], []);
    if actor.use_internal_behavior {
        internal_behavior(actor, rx, state).await
    } else {
        actor.simulated_behavior(sim_runners!(rx)).await
    }
}

pub(crate) async fn internal_behavior<C: SteadyActor>(mut actor: C, rx: SteadyRx<ApprovedWidgets>, state: Arc<Mutex<InternalState>>) -> Result<(), Box<dyn Error>> {
    //let args:Option<&Args> = context.args(); //you can make the type explicit
    let _args = actor.args::<Args>(); //or you can turbo fish here to get your args
    //trace!("running {:?} {:?}",context.id(),context.name());

    let mut rx = rx.lock().await;
    let mut state = state.lock().await;

    //predicate which affirms or denies the shutdown request
    while actor.is_running(&mut || rx.is_closed_and_empty()) {
        let _clean = await_for_all!(actor.wait_avail(&mut rx,1));

        //example of high volume processing, we stay here until there is no more work BUT
        //we must also relay our telemetry data periodically
        while !rx.is_empty() {
            let mut buf = state.buffer;
            let count = *actor.take_slice(&mut rx, &mut buf);
            for item in buf[..count].iter() {
                state.last_approval = Some(item.to_owned());
            }
            //based on the channel capacity this will send batched updates so most calls do nothing.
            actor.relay_stats_smartly();

        }
    }

    Ok(())
}

#[cfg(test)]
mod consumer_tests {
    use std::time::Duration;
    use super::*;

    #[test]
    fn test_consumer() -> Result<(), Box<dyn Error>> {
        // build test graph, the input and output channels and our actor
        let mut graph = GraphBuilder::for_testing().build(());

        let (approved_widget_tx_out,approved_widget_rx_out) = graph.channel_builder()
            .with_capacity(BATCH_SIZE).build_channel();

        let state = Arc::new(Mutex::new(InternalState::new()));

        graph.actor_builder()
            .with_name("UnitTest")
            .build(move |context| internal_behavior(context, approved_widget_rx_out.clone(), state.clone()), SoloAct);

        graph.start();
        graph.request_shutdown();

        let test_data: Vec<ApprovedWidgets> = (0..BATCH_SIZE).map(|i| ApprovedWidgets { original_count: 0, approved_count: i as u64 }).collect();
        approved_widget_tx_out.testing_send_all(test_data, true);

        graph.block_until_stopped(Duration::from_secs(240))

    }

}
