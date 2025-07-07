use std::error::Error;
#[allow(unused_imports)]
use log::*;
use steady_state::*;
use steady_state::state_management::SteadyState;
use steady_state::SteadyRx;
use crate::actor::data_generator::Packet;

pub struct ProcessorState {
    pub count: u64,
}

pub async fn run(actor: SteadyActorShadow, rx: SteadyRx<Packet>, state: SteadyState<ProcessorState>) -> Result<(), Box<dyn Error>> {
    let actor = actor.into_spotlight([&rx], []);
    if actor.use_internal_behavior {
        internal_behavior(actor, rx, state).await
    } else {
        actor.simulated_behavior(vec![&rx]).await
    }
}

async fn internal_behavior<C: SteadyActor>(mut actor: C, rx: SteadyRx<Packet>, state: SteadyState<ProcessorState>) -> Result<(), Box<dyn Error>> {
    let mut state = state.lock(|| ProcessorState { count: 0 }).await;
    let mut rx = rx.lock().await;
    while actor.is_running(|| rx.is_closed_and_empty()) {
        await_for_all!(actor.wait_avail(&mut rx, 1));
        while let Some(packet) = actor.try_take(&mut rx) {
            assert_eq!(packet.data.len(), 62);
            state.count += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use steady_state::GraphBuilder;
    use steady_state::state_management::new_state;
    use std::time::Duration;
    use bytes::Bytes;
    use crate::actor::data_generator::Packet;

    #[test]
    fn test_processor() -> Result<(), Box<dyn Error>> {

        //get the only copy of the state so we can unwrap it for testing.
            let state = new_state();
            let test_state = state.clone();

            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, rx) = graph.channel_builder().build();

            graph.actor_builder()
                .with_name("UnitTest")
                .build(  move|context| internal_behavior(context, rx.clone(), state.clone()), SoloAct);

            graph.start();

            let test_packets = vec![
                Packet { route: 0, data: Bytes::from(vec![0u8; 62]) },
                Packet { route: 0, data: Bytes::from(vec![0; 62]) },
                Packet { route: 0, data: Bytes::from(vec![0; 62]) },
            ];
            tx.testing_send_all(test_packets, true);

            graph.request_shutdown();
            graph.block_until_stopped(Duration::from_secs(1))?;


        //TODO: need distributed workd solution,
        //      - new workers broadcast their loction to be routed to
        //      - we drop them after failures. and route elsewhere.
        //TODO: rename wait method?



        //TODO: add this example somewhere ??
        //actor.call_async    actor.call_blocking  - turn blocking into async call.

        //TODO: add this test solution to the standard example...
        let final_state = test_state.try_lock_sync().expect("Couldn't lock state");
        assert_eq!(final_state.count, 3);

        Ok(())
    }
}