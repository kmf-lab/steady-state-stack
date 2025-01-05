use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use futures_util::lock::Mutex;
use log::warn;
use crate::channel_builder::ChannelBuilder;
use crate::{Rx, SteadyCommander, Tx};
use crate::monitor::{RxMetaData, TxMetaData};


// pub trait ToSerial {
//     fn serialize_into<CMD: SteadyCommander>(self, cmd: &mut CMD, buffer: &mut Tx<u8>) -> Option<usize>; //length of bytes
// }
// pub trait FromSerial {
//     fn deserialize_from<CMD: SteadyCommander>(cmd: &mut CMD, buffer: &mut Rx<u8>, meta: AqueductMetadata) -> Self;
// }
// pub trait CloneSerial {
//     fn deserialize_from<CMD: SteadyCommander>(cmd: &mut CMD, buffer: &mut Rx<u8>, meta: AqueductMetadata) -> Self;
// }
// pub trait PeekSerial<'a, 'b> {
//     fn deserialize_from<CMD: SteadyCommander>(cmd: &'b mut CMD, buffer: &'a mut Rx<u8>, meta: AqueductMetadata) -> &'a Self;
// }


pub fn build_aqueduct(control_builder: &ChannelBuilder, payload_builder: &ChannelBuilder) -> ( LazyAqueductTx, LazyAqueductRx) {
    let to_aeron   = Arc::new(LazyAqueduct::new(&control_builder, &payload_builder));
    (LazyAqueductTx::new(to_aeron.clone()),LazyAqueductRx::new(to_aeron.clone()))
}


type IdType = i32; // is i32 because this what aeron is using, unlikely to change


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentType {
    Begin,
    Middle,
    End,
    UnFragmented,
}

#[derive(Clone, Copy, Debug)]
pub enum FragmentDirection {
    Outgoing(),
    Incoming(IdType, Instant), //incoming gets sessionId plus arrival time
}


#[derive(Clone, Copy, Debug)]
pub struct AqueductFragment {
    pub(crate) fragment_type: FragmentType,
    pub(crate) length: i32,
    pub(crate) option: Option<i64>,
    pub(crate) stream_id: IdType,
    pub(crate) direction: FragmentDirection,
}
impl AqueductFragment {
    pub fn new(fragment_type: FragmentType, stream_id: IdType, direction: FragmentDirection, length: i32, option: Option<i64>) -> Self {
        AqueductFragment {
            fragment_type,
            length,
            option,
            stream_id,
            direction            
        }
    }
    pub fn simple_outgoing(stream_id: IdType, length: i32) -> Self {
        AqueductFragment {
            fragment_type: FragmentType::UnFragmented,
            length,
            option: None,
            stream_id,
            direction: FragmentDirection::Outgoing()
        }
    }
}


pub(crate) struct MessageCollector {
   pub stream_id: IdType,
   pub session_id: IdType, 
   pub began: Instant, //time when head arrived from aeron poll
   pub finish: Option<Instant>, //time when tail arrived on consumer end of aqueduct
   pub option: Option<i64>,
   pub data: Vec<u8> 
}
pub struct AqueductRx {
    pub(crate) control_channel: Rx<AqueductFragment>,
    pub(crate) payload_channel: Rx<u8>,
    pub(crate) assembly: HashMap<IdType, HashMap<IdType, Vec<MessageCollector>>>, //stream map, session map, vec collectors
}

impl AqueductRx {
    pub fn new(control_channel: Rx<AqueductFragment>, payload_channel: Rx<u8>) -> Self {
        AqueductRx {
            control_channel,
            payload_channel,
            assembly: HashMap::new(),
        }
    }

    pub fn take_stream_unordered(&mut self, stream_id: i32) -> Vec<MessageCollector> {
        let mut result = Vec::new();

        if let Some(session_map) = self.assembly.get_mut(&stream_id) {
            // Collect session keys to remove if they become empty
            let mut keys_to_remove = Vec::new();

            for (session_id, vec) in session_map.iter_mut() {
                let mut i = 0;
                while i < vec.len() {
                    if vec[i].finish.is_some() {
                        // Remove the collector and transfer ownership to the result
                        result.push(vec.remove(i));
                    } else {
                        i += 1; // Only increment if we didn't remove an element
                    }
                }

                // Mark the session key for removal if its vector is now empty
                if vec.is_empty() {
                    keys_to_remove.push(*session_id);
                }
            }

            // Remove session keys with empty vectors
            for key in keys_to_remove {
                session_map.remove(&key);
            }

            // Remove the parent key if the session map is now empty
            if session_map.is_empty() {
                self.assembly.remove(&stream_id);
            }
        }

        result
    }

    pub fn take_stream_arrival(&mut self, stream_id: i32) -> Vec<MessageCollector> {
        let mut result = Vec::new();

        if let Some(session_map) = self.assembly.get_mut(&stream_id) {
            // Collect session keys to remove if they become empty
            let mut keys_to_remove = Vec::new();

            // Create a vector of mutable references to each session's vector
            let mut session_iters: Vec<_> = session_map
                .iter_mut()
                .map(|(session_id, vec)| (session_id, vec))
                .collect();

            // Perform the merge-like process
            while !session_iters.is_empty() {
                // Find the next `FrameCollector` with the smallest `began` time
                let mut smallest_idx = 0;
                for i in 1..session_iters.len() {
                    if session_iters[i].1[0].began < session_iters[smallest_idx].1[0].began {
                        smallest_idx = i;
                    }
                }

                // Remove the smallest collector and push it to the result
                let (session_id, vec) = &mut session_iters[smallest_idx];
                result.push(vec.remove(0));

                // If the session's vector is empty, mark it for removal
                if vec.is_empty() {
                    keys_to_remove.push(**session_id);
                    session_iters.remove(smallest_idx);
                }
            }

            // Remove empty session keys from the session map
            for key in keys_to_remove {
                session_map.remove(&key);
            }

            // Remove the parent key if the session map is now empty
            if session_map.is_empty() {
                self.assembly.remove(&stream_id);
            }
        }
        result
    }




    /// non blocking call to consume available data and move it to vecs
    /// returns count of fragments consumed
    pub fn defragment<T: SteadyCommander>(&mut self, cmd: &mut T) -> usize {

        //TODO: check the tree and see how much memory we are using if this is getting out of hand do not process anything new.
        

        //how many fragments will we attempt to process at once is limited here
        let mut frags = [AqueductFragment::simple_outgoing(-1, 0); 100];
        let frags_count = cmd.take_slice(&mut self.control_channel, &mut frags);
        for i in 0..frags_count {
            let frame = frags[i];
            if let FragmentDirection::Incoming(session_id, arrival)  = frame.direction {
                match frame.fragment_type {
                    FragmentType::UnFragmented | FragmentType::Begin => {
                        let mut data = vec![0u8; frame.length as usize];
                        let count = cmd.take_slice(&mut self.payload_channel, &mut data);
                        debug_assert_eq!(count, frame.length as usize);

                        let session_vec = self.assembly.entry(frame.stream_id).or_insert(HashMap::new())
                            .entry(session_id).or_insert(Vec::new());
                        #[cfg(debug_assertions)]
                        if session_vec.len() != 0 {
                            debug_assert_eq!(true, session_vec.last_mut().expect("vec").finish.is_some());
                            //TODO: we probably need to force close this?? based on some timeout.
                        }
                        session_vec.push(MessageCollector {
                            stream_id: frame.stream_id
                            ,
                            option: frame.option
                            ,
                            began: arrival
                            ,
                            finish: if frame.fragment_type == FragmentType::UnFragmented { Some(Instant::now()) } else { None }
                            ,
                            data,
                            session_id: 0,
                        });
                    },
                    FragmentType::Middle | FragmentType::End => {
                        let mut session_vec = self.assembly.entry(frame.stream_id).or_insert(HashMap::new())
                            .entry(session_id).or_insert(Vec::new());
                        let mut collector = session_vec.last_mut().expect("vec");
                        debug_assert_eq!(false, collector.finish.is_some());
                        let start = collector.data.len();
                        collector.data.resize(collector.data.len() + frame.length as usize, 0);
                        let count = cmd.take_slice(&mut self.payload_channel, &mut collector.data[start..]);
                        debug_assert_eq!(count, frame.length as usize);
                        if frame.fragment_type == FragmentType::End {
                            collector.finish = Some(Instant::now());
                        }
                        debug_assert_eq!(collector.option, frame.option, "Reserved Option i64 must not change inside message fragments");
                    }
                }
            } else {
                warn!("Expected FrameDirection::Incoming");
            }
        }
        frags_count        
    }
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
    pub(crate) control_channel: Tx<AqueductFragment>,
    pub(crate) payload_channel: Tx<u8>,
}
impl AqueductTx {
    pub fn new(control_channel: Tx<AqueductFragment>, payload_channel: Tx<u8>) -> Self {
        AqueductTx {
            control_channel,
            payload_channel,
        }
    }
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
    
    // // TODO: we may determine that this timestamp is not needed.
    // pub async fn try_send<CMD: SteadyCommander, T: ToSerial>(&mut self,  cmd: &mut CMD, message: T, bonus: u64) -> bool {
    //     if self.control_channel.shared_vacant_units()>0 {
    //         let length: Option<usize> = message.serialize_into(cmd, &mut self.payload_channel);
    //         assert_ne!(length, Some(0));
    //         if let Some(bytes_count) = length {
    //             let _ = cmd.try_send(&mut self.control_channel, AqueductMetadata { bytes_count, bonus });
    //             true
    //         } else {
    //             false
    //         }
    //     } else {
    //         false
    //     }
    // }
    
    //TODO:add other methods..
    
}




impl AqueductRx {

    pub fn is_closed_and_empty(&mut self) -> bool {
        self.control_channel.is_closed_and_empty() &&
        self.payload_channel.is_closed_and_empty()
    }



    async fn take_or_peek(&mut self, stream_id: i32 ) {
        //defrag
        //if channel needs more data for us await
        //if next chunk goes to vec which holds message we could become blocked !!

    }

    //not going to work until we defrag the data
    // pub async fn try_take<CMD: SteadyCommander, T:FromSerial
    // >(&mut self, cmd: &mut CMD) -> Option<T> {
    //     match cmd.try_take(&mut self.control_channel) {
    //         Some(meta) => {
    //             Some(T::deserialize_from(cmd, &mut self.payload_channel, meta))
    //         },
    //         None => None
    //     }
    // }

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
           assert_eq!(c.length as usize, data.len());
           let count = l.payload_channel.shared_take_slice(data);
           assert_eq!(count, c.length as usize);
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
        match l.control_channel.shared_try_send(AqueductFragment::simple_outgoing(1, x as i32 )) {
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
            let tx = Arc::new(Mutex::new(AqueductTx::new(meta_tx, data_tx )));
            let rx = Arc::new(Mutex::new(AqueductRx::new(meta_rx, data_rx)));
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
            let tx = Arc::new(Mutex::new(AqueductTx::new(meta_tx, data_tx)));
            let rx = Arc::new(Mutex::new(AqueductRx::new(meta_rx, data_rx)));
            *channel = Some((tx,rx));
        }
        channel.as_ref().expect("internal error").1.clone()
    }
}