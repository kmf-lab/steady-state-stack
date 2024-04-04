use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;
use bastion::context::BastionContext;
use bastion::message::MessageHandler;

use log::error;
use futures::channel::oneshot::Receiver;
use crate::Graph;

#[derive(Debug)]
pub enum GraphTestResult<K,E>
    where
        K: Any+Send+Sync+Debug,
        E: Any+Send+Sync+Debug{
    Ok(K),
    Err(E),
}

pub struct EdgeSimulator {
    pub(crate) ctx: Arc<BastionContext>,
}

impl EdgeSimulator {
    pub fn new(ctx: Arc<BastionContext>) -> Self {
        EdgeSimulator {
            ctx,
        }
    }

    pub async fn respond_to_request<F, T: 'static, K, E>(&self, mut f: F)
        where
            F: FnMut(T) -> GraphTestResult<K, E>,
            K: Any+Send+Sync+Debug,
            E: Any+Send+Sync+Debug,
    {
        match self.ctx.recv().await {
            Ok(m) => {
                MessageHandler::new(m)
                    .on_question(move |message: T, answer_sender| {
                        // Using async block to capture the future returned by x and then executing it.
                        let result:GraphTestResult<K, E> = f(message);
                        // Send the result back using answer_sender.
                        match answer_sender.reply(result) {
                            Ok(_) => {},
                            Err(e) => {
                                error!("Error sending test implementation response: {:?}", e);
                            }
                        };
                    });
            }
            Err(e) => {
                error!("Error receiving test message: {:?}", e);
            }
        }

    }

}

pub struct EdgeSimulationDirector {
    distributor: bastion::distributor::Distributor,
}

impl EdgeSimulationDirector {
    pub fn new(_graph: &Graph, name: & 'static str) -> EdgeSimulationDirector {
        EdgeSimulationDirector {
            distributor: bastion::distributor::Distributor::named(name)
        }
    }

    pub async fn send_request<T, K, E>(&self, msg: T) -> Option<GraphTestResult<K, E>>
      where T: Any + Send + Sync + Debug + Clone + 'static,
            K: Any + Send + Sync + Debug,
            E: Any + Send + Sync + Debug
    {
        let answer_consumer:Receiver<Result<GraphTestResult<K,E>,bastion::errors::SendError>> = self.distributor.request(msg);
        match answer_consumer.await {
            Ok(ac) => {
                match ac {
                    Ok(response) => {
                        return Some(response);
                    },
                    Err(e) => {
                        error!("failed to send request: {:?}", e);
                    }
                }
            }
            Err(e) => {
                error!("failed to send request: {:?}", e);
            }
        }
       None
    }

}


