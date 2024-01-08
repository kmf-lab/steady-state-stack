use std::sync::Arc;
use std::time::Duration;
use crate::steady::*;


#[derive(Clone, Debug)]
pub enum DiagramData {

    //Structure(),
    //Content(),

}

struct InternalState {
    pub child_group_name: Vec<& 'static str>,
    pub index_received: Vec<Vec<usize>>,
    pub index_sent: Vec<Vec<usize>>,

    total_sent: Vec<usize>,
    total_received: Vec<usize>,
    child_group_name_dirty: bool
}

pub(crate) async fn run(mut monitor: SteadyMonitor
                        , all_rx: Arc<RwLock<Vec< CollectorDetail >>>
                        , outgoing: Vec<SteadyTx<DiagramData>>
) -> Result<(),()> {

    //TODO: may need custom method for this
    //let rx_def = all_rx.read().await.as_slice();
    //monitor.init_stats(rx_def, &outgoing);


    let mut state = InternalState {
        child_group_name: Vec::new(),
        index_received: Vec::new(),
        index_sent: Vec::new(),
        total_received: Vec::new(),
        total_sent: Vec::new(),
        child_group_name_dirty: false

    };

    let outgoing = outgoing.clone();
    loop {

        let all = all_rx.read().await;

        let mut count_down:u8 = 20; //max checks before we break out of the loop

        /*
        loop {


            let v: Vec<& CollectorDetail> = all.iter()
                .filter(|x| x.telemetry_rx.has_message())
                .map(|x| &x.telemetry_rx)
                .collect();

            count_down += 1;
            if v.is_empty() || 0 == count_down {
                break;
            }

            for tel in v {
                match monitor.rx(&tel).await {

                    Ok(_) => {
                        //we do not care about other telemetry

                    },

                    /*
                    Ok(Telemetry::Messages(tx_channel_values, rx_channel_values)) => {

                        //these are sent very frequently and are the actual data
                        state.index_sent[tel.id].iter()
                            .zip(tx_channel_values.iter())
                            .for_each(|(index, value)| {
                                if *index >= state.total_sent.len() {
                                    state.total_sent.resize_with(*index + 1, || 0);
                                }
                                state.total_sent[*index] += value;
                            });

                        state.index_received[tel.id].iter()
                            .zip(rx_channel_values.iter())
                            .for_each(|(index, value)| {
                                if *index >= state.total_received.len() {
                                    state.total_received.resize_with(*index + 1, || 0);
                                }
                                state.total_received[*index] += value;
                            });
                    },
                    */

                    Err(e) => {
                        error!("Unexpected error recv_async: {}",e);
                    }
                }
            }
        }
       //    */

        if state.child_group_name_dirty {
            if outgoing.iter().all(|x| x.has_room()) {
                for tx in outgoing.clone() {
                    //TODO: reevaulate this data structure, can we get channel related data?
                    //monitor.tx(&tx, DiagramData::Structure(state.child_group_name)).await;
                }
            }
        }
        if outgoing.iter().all(|x| x.has_room()) {
            for tx in outgoing.clone() {
                //we have a sent count for each sender
                //we ahve a recieve count for each reciever


                //TODO: reevaulate this data structure
                //monitor.tx(&tx, DiagramData::Content(state.total_sent,state.total_receved)).await;
            }
        }

        Delay::new(Duration::from_millis(5)).await;
        if false {
            break Ok(());
        }
    }
}