use std::error::Error;
use std::fmt::Debug;
use log::error;
use crate::{SteadyCommander, SteadyContext, SteadyTx};
use crate::SteadyRx;


pub enum Simulate<T, C: SteadyCommander> {
    Echo(C,SteadyTx<T>),
    Equals(C,SteadyRx<T>),
}

pub async fn external_behavior<T: Clone + Debug + Send + Eq, C: SteadyCommander >(testing: Simulate<T, C>) -> Result<(), Box<dyn Error>> {
    match testing {
        Simulate::Echo(mut cmd, tx) => {
            if let Some(responder) = cmd.sidechannel_responder() {
                let mut heartbeat_tx = tx.lock().await;
                while cmd.is_running(&mut || heartbeat_tx.mark_closed()) {
                    // in main use graph.sidechannel_director node_call(msg,"heartbeat")
                    if !responder.echo_responder(&mut cmd, &mut heartbeat_tx).await {
                        //this failure should not happen
                        error!("Unable to send simulated heartbeat for graph test");
                    }
                }
            }
        }
        Simulate::Equals(mut cmd, rx) => {
            if let Some(responder) = cmd.sidechannel_responder() {
                let mut fizz_buzz_rx = rx.lock().await;
                while cmd.is_running(&mut ||fizz_buzz_rx.is_closed_and_empty()) {
                    // in main use graph.sidechannel_director node_call(msg,"heartbeat")
                    if !responder.equals_responder(&mut cmd, &mut fizz_buzz_rx).await {
                        //this failure should not happen
                        error!("Unable to send simulated heartbeat for graph test");
                    }
                }
            }
        }

    }

    Ok(())
}