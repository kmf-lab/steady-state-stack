use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::DerefMut;
use std::sync::Arc;
use async_ringbuf::AsyncRb;
use async_ringbuf::consumer::AsyncConsumer;
use async_ringbuf::producer::AsyncProducer;

use log::error;
use futures_util::lock::{Mutex};
use async_ringbuf::traits::Split;

use crate::channel_builder::{ChannelBacking, InternalReceiver, InternalSender};

#[derive(Debug)]
pub enum GraphTestResult<K,E>
    where
        K: Any+Send+Sync+Debug,
        E: Any+Send+Sync+Debug{
    Ok(K),
    Err(E),
}



pub(crate) type SideChannel = (InternalSender<Box<dyn Any+Send+Sync>>, InternalReceiver<Box<dyn Any+Send+Sync>>);
type NodeName = String;

pub struct SideChannelHub {
    //each node holds its own lock on read and write to the backplane
    node: HashMap<NodeName, Arc<Mutex<SideChannel>>  >,
    //the backplane can only be held by one user at a time and functions as a central message hub
    pub(crate) backplane: HashMap<NodeName, SideChannel >,
}


impl SideChannelHub {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        SideChannelHub {
            node: Default::default(),
            backplane: HashMap::new(),
        }
    }

    pub fn node_tx_rx(&self, name: &str) -> Option<Arc<Mutex<SideChannel>>> {
        self.node.get(name).cloned()
    }

    pub fn register_node(&mut self, name: &str, capacity: usize){
        //message to the node
        let rb = AsyncRb::<ChannelBacking<Box<dyn Any+Send+Sync>>>::new(capacity);
        let (sender_tx, receiver_tx) = rb.split();

        //response from the node
        let rb = AsyncRb::<ChannelBacking<Box<dyn Any+Send+Sync>>>::new(capacity);
        let (sender_rx, receiver_rx) = rb.split();

        self.node.insert(name.into(), Arc::new(Mutex::new((sender_rx, receiver_tx))));
        self.backplane.insert(name.into(), (sender_tx, receiver_rx));
    }


    pub async fn node_call(&mut self, msg: Box<dyn Any+Send+Sync>, name: &str) -> Option<Box<dyn Any+Send+Sync>> {
        if let Some((tx, rx)) = self.backplane.get_mut(name) {
            match tx.push(msg).await {
                Ok(_) => {
                    return rx.pop().await;
                },
                Err(e) => {
                    error!("Error sending test implementation request: {:?}", e);
                }
            }
        }
        None
    }
}



pub struct SideChannelResponder {
    pub(crate) arc: Arc<Mutex<SideChannel>>
}

impl SideChannelResponder {
    pub fn new(
               arc: Arc<Mutex<SideChannel>>
    ) -> Self {
        SideChannelResponder {
            arc
        }
    }

    pub async fn respond_with<F>(&self, mut f: F)
        where
            F: FnMut(Box<dyn Any+Send+Sync>) -> Box<dyn Any+Send+Sync>,
    {
        let mut guard = self.arc.lock().await;
        let (ref mut tx, ref mut rx) = guard.deref_mut();
        if let Some(msg) =rx.pop().await {
                match tx.push(f(msg)).await {
                    Ok(_) => {},
                    Err(e) => {
                        error!("Error sending test implementation response: {:?}", e);
                    }
                };
        }
    }

}


