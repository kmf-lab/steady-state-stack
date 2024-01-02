use std::sync::Arc;
use std::time::Duration;
use crate::steady::*;


#[derive(Clone, Debug)]
pub enum DiagramData {

    Structure(),
    Content(),

}

struct InternalState {
    pub node_names: Vec<String>,
    pub index_received: Vec<Vec<usize>>,
    pub index_sent: Vec<Vec<usize>>,

}

pub(crate) async fn run(mut monitor: SteadyMonitor
                        , all_rx: Arc<RwLock<Vec< SteadyRx<Telemetry> >>>
                        , tx: SteadyTx<DiagramData>
) -> Result<(),()> {

    let mut state = InternalState {
        node_names: Vec::new(),
        index_received: Vec::new(),
        index_sent: Vec::new(),

    };

  //  state.data.iter().for_each(|x| {
  //      info!("data: {:?}", x);
  //  });

        //TODO: need to send this each time it changes to the server (reqiress common super)
        monitor.tx(&tx, DiagramData::Structure()).await;
        //TODO: need to send this once every 20ms to the server.
        monitor.tx(&tx, DiagramData::Content()).await;

    loop {

        let all = all_rx.read().await;
        let v:Vec<&SteadyRx<Telemetry>> = all.iter()
                                             .filter(|x| x.has_message())
                                             .collect();
        for tel in v {
            while tel.has_message() {
                match monitor.rx(&tel).await {
                    Ok(Telemetry::ActorDef(name)) => {
                        if tel.id < state.node_names.len() {
                            state.node_names[tel.id] = name.to_string();
                        } else {
                            state.node_names.resize_with(tel.id + 1, String::new);
                            state.node_names[tel.id] = name.to_string();
                        }
                    },
                    Ok(Telemetry::MessagesIndex(tx_channel_ids, rx_channel_ids)) => {
                        if tel.id < state.index_received.len() {
                            state.index_received[tel.id] = rx_channel_ids;
                        } else {
                            state.index_received.resize_with(tel.id + 1, Vec::new);
                            state.index_received[tel.id] = rx_channel_ids;
                        }
                        if tel.id < state.index_sent.len() {
                            state.index_sent[tel.id] = tx_channel_ids;
                        } else {
                            state.index_sent.resize_with(tel.id + 1, Vec::new);
                            state.index_sent[tel.id] = tx_channel_ids;
                        }
                    },
                    Ok(Telemetry::Messages(tx_channel_values, rx_channel_values)) => {

                        let _s_index = state.index_sent[tel.id]
                                            .iter()
                                            .zip(tx_channel_values.iter());

                        let _r_index = state.index_received[tel.id]
                                            .iter()
                                            .zip(rx_channel_values.iter());

                        //TODO: for each sum them in the arrays of totals

                        //TODO: when done exit this channel so we can capture the others before returning
                        //look up the index for this channel_rx_id and store the data
                        info!("Got MessagesReceived: {:?} use index to store data", rx_channel_values);
                    },
                    Err(e) => {
                        error!("Unexpected error recv_async: {}",e);
                    }
                }
            }
        }

        //TODO: if 10ms have passed send the totals to the next actor



        Delay::new(Duration::from_millis(20)).await;
        if false {
            break Ok(());
        }
    }
}