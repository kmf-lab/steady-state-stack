use std::sync::Arc;
use std::time::Duration;
use futures_util::lock::Mutex;
use log::warn;
use crate::channel_builder::ChannelBuilder;
use crate::{Rx, SteadyCommander, Tx};
use crate::distributed::aeron_channel::Channel;
use crate::monitor::{RxMetaData, TxMetaData};


pub trait ToSerial {
    fn serialize_into<CMD: SteadyCommander>(self, cmd: &mut CMD, buffer: &mut Tx<u8>) -> Option<usize>; //length of bytes
}
pub trait FromSerial {
    fn deserialize_from<CMD: SteadyCommander>(cmd: &mut CMD, buffer: &mut Rx<u8>, meta: AqueductMetadata) -> Self;
}
pub trait CloneSerial {
    fn deserialize_from<CMD: SteadyCommander>(cmd: &mut CMD, buffer: &mut Rx<u8>, meta: AqueductMetadata) -> Self;
}
pub trait PeekSerial<'a, 'b> {
    fn deserialize_from<CMD: SteadyCommander>(cmd: &'b mut CMD, buffer: &'a mut Rx<u8>, meta: AqueductMetadata) -> &'a Self;
}


#[derive(Clone, Copy, Debug)]
pub struct AqueductMetadata {
    pub(crate) bytes_count: usize,
    pub(crate) bonus: u64 //extra??
}

pub struct AqueductRx {
    pub(crate) control_channel: Rx<AqueductMetadata>,
    pub(crate) payload_channel: Rx<u8>,
}
pub type SteadyAqueductRx = Arc<Mutex<AqueductRx>>;

pub trait AquaductRxDef {
    fn meta_data(self: &Self) -> AquaductRxMetaData;
}

pub struct AquaductRxMetaData {
    pub(crate) control: RxMetaData,
    pub(crate) payload: RxMetaData,
}

impl AquaductRxDef for SteadyAqueductRx {
    fn meta_data(self: &Self) -> AquaductRxMetaData {
        match self.try_lock() {
            Some(rx_lock) => {
                let m1 = RxMetaData(rx_lock.control_channel.channel_meta_data.clone());
                let d2 = RxMetaData(rx_lock.payload_channel.channel_meta_data.clone());
                AquaductRxMetaData{control:m1,payload:d2}
            },
            None => { 
                let rx_lock = nuclei::block_on(self.lock());
                let m1 = RxMetaData(rx_lock.control_channel.channel_meta_data.clone());
                let d2 = RxMetaData(rx_lock.payload_channel.channel_meta_data.clone());
                AquaductRxMetaData{control:m1,payload:d2}
            }
        }
    }
}

pub struct AqueductTx {
    pub(crate) control_channel: Tx<AqueductMetadata>,
    pub(crate) payload_channel: Tx<u8>,
}
pub type SteadyAqueductTx = Arc<Mutex<AqueductTx>>;
pub trait AquaductTxDef {
    fn meta_data(self: &Self) -> AquaductTxMetaData;
}
pub struct AquaductTxMetaData {
    pub(crate) control: TxMetaData,
    pub(crate) payload: TxMetaData,
}
impl AquaductTxDef for SteadyAqueductTx {
    fn meta_data(self: &Self) -> AquaductTxMetaData {
        match self.try_lock() {
            Some(rx_lock) => {
                let m1 = TxMetaData(rx_lock.control_channel.channel_meta_data.clone());
                let d2 = TxMetaData(rx_lock.payload_channel.channel_meta_data.clone());
                AquaductTxMetaData{control:m1,payload:d2}
            },
            None => {
                let rx_lock = nuclei::block_on(self.lock());
                let m1 = TxMetaData(rx_lock.control_channel.channel_meta_data.clone());
                let d2 = TxMetaData(rx_lock.payload_channel.channel_meta_data.clone());
                AquaductTxMetaData{control:m1,payload:d2}
            }
        }
    }
}

impl AqueductTx {

    pub fn mark_closed(&mut self) -> bool {
        self.control_channel.mark_closed();
        self.payload_channel.mark_closed();
        true
    }
    
    // TODO: we may determine that this timestamp is not needed.
    pub async fn try_send<CMD: SteadyCommander, T: ToSerial>(&mut self,  cmd: &mut CMD, message: T, bonus: u64) -> bool {
        if self.control_channel.shared_vacant_units()>0 {
            let length: Option<usize> = message.serialize_into(cmd, &mut self.payload_channel);
            assert_ne!(length, Some(0));
            if let Some(bytes_count) = length { 
                let _ = cmd.try_send(&mut self.control_channel, AqueductMetadata { bytes_count, bonus });
                true
            } else {
                false
            }
        } else {
            false
        }
    }
    
    //TODO:add other methods..
    
}

impl AqueductRx {

    pub fn is_closed_and_empty(&mut self) -> bool {
        self.control_channel.is_closed_and_empty() &&
        self.payload_channel.is_closed_and_empty()
    }
    
    pub async fn try_take<CMD: SteadyCommander, T:FromSerial
    >(&mut self, cmd: &mut CMD) -> Option<T> {
        match cmd.try_take(&mut self.control_channel) {
            Some(meta) => {
                Some(T::deserialize_from(cmd, &mut self.payload_channel, meta))
            },
            None => None
        }
    }

    //TODO:add other methods..

}

pub struct LazyAqueductRx {
    lazy_channel: Arc<LazyAqueduct>,
}

impl LazyAqueductRx {
    pub(crate) fn new(lazy_channel: Arc<LazyAqueduct>) -> Self {
        LazyAqueductRx {
            lazy_channel,
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyAqueductRx {
        nuclei::block_on(self.lazy_channel.get_rx_clone())
    }

    pub async fn testing_take_frame(&self, data: &mut [u8]) -> usize {
        let s = self.clone();
        let mut l = s.lock().await;
        if let Some(c) = l.control_channel.shared_take_async().await {
           assert_eq!(c.bytes_count, data.len());
           let count = l.payload_channel.shared_take_slice(data);
           assert_eq!(count, c.bytes_count);
           count
        } else {
            warn!("error taking metadata");
            0
        }
    }


}

pub struct LazyAqueductTx {
    lazy_channel: Arc<LazyAqueduct>,
}

impl LazyAqueductTx {
    pub(crate) fn new(lazy_channel: Arc<LazyAqueduct>) -> Self {
        LazyAqueductTx {
            lazy_channel,
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyAqueductTx {
        nuclei::block_on(self.lazy_channel.get_tx_clone())
    }

    pub async fn testing_send_frame(&self, data: &[u8]) {
        let s = self.clone();
        let mut l = s.lock().await;
        let x = l.payload_channel.shared_send_slice_until_full(data);
        assert_eq!(x, data.len()); //for this test we must send it all
        assert_ne!(x, 0);
        match l.control_channel.shared_try_send(AqueductMetadata { bytes_count: x, bonus: 0 }) {
            Ok(_) => {},
            Err(_) => { panic!("error sending metadata"); }
        };
    }
    pub async fn testing_close(&self) {
        let s = self.clone();
        let mut l = s.lock().await;
        l.payload_channel.mark_closed();
        l.control_channel.mark_closed();
    }

}

#[derive(Debug)]
pub(crate) struct LazyAqueduct {
    control_builder: Mutex<Option<ChannelBuilder>>,
    payload_builder: Mutex<Option<ChannelBuilder>>,
    channel: Mutex<Option<(SteadyAqueductTx, SteadyAqueductRx)>>,
}

impl LazyAqueduct {
    pub(crate) fn new(control_builder: &ChannelBuilder, payload_builder: &ChannelBuilder) -> Self {
        LazyAqueduct {
            control_builder: Mutex::new(Some(control_builder.clone())),
            payload_builder: Mutex::new(Some(payload_builder.clone())),
            channel: Mutex::new(None),
        }
    }

    pub(crate) async fn get_tx_clone(&self) -> SteadyAqueductTx {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let mut meta_builder = self.control_builder.lock().await.take().expect("internal error");
            let mut data_builder = self.payload_builder.lock().await.take().expect("internal error");
            let (meta_tx,meta_rx) = meta_builder.eager_build_internal();
            let (data_tx,data_rx) = data_builder.eager_build_internal();
            let tx = Arc::new(Mutex::new(AqueductTx { control_channel: meta_tx, payload_channel: data_tx }));
            let rx = Arc::new(Mutex::new(AqueductRx { control_channel: meta_rx, payload_channel: data_rx }));
            *channel = Some((tx,rx));
        }
        channel.as_ref().expect("internal error").0.clone()
    }

    pub(crate) async fn get_rx_clone(&self) -> SteadyAqueductRx {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let mut meta_builder = self.control_builder.lock().await.take().expect("internal error");
            let mut data_builder = self.payload_builder.lock().await.take().expect("internal error");
            let (meta_tx,meta_rx) = meta_builder.eager_build_internal();
            let (data_tx,data_rx) = data_builder.eager_build_internal();
            let tx = Arc::new(Mutex::new(AqueductTx { control_channel: meta_tx, payload_channel: data_tx }));
            let rx = Arc::new(Mutex::new(AqueductRx { control_channel: meta_rx, payload_channel: data_rx }));
            *channel = Some((tx,rx));
        }
        channel.as_ref().expect("internal error").1.clone()
    }
}