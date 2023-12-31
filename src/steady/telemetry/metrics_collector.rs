use std::sync::Arc;
use std::time::Duration;
use crate::steady::*;


#[derive(Clone, Debug)]
pub enum DiagramData {

    Structure(),
    Content(),

}

struct InternalState {
    pub data: Vec<DiagramData>
}

pub(crate) async fn run(mut monitor: SteadyMonitor
                        , all_rx: Arc<RwLock<Vec< SteadyRx<Telemetry> >>>
                        , tx: SteadyTx<DiagramData>
) -> Result<(),()> {

    let state = InternalState { data: Vec::new() };

    state.data.iter().for_each(|x| {
        info!("data: {:?}", x);
    });

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
                if true {
                    break;
                }

                match monitor.rx(&tel).await { //TODO: need to handle out of bounds error
                    Ok(Telemetry::ActorDef(m)) => {
                        info!("Got ActorDef: {:?} define the node", m);
                    },
                    Ok(Telemetry::MessagesIndexReceived(m)) => {
                    //     tel.index_received = Some(m);
                     //   let index:Vec<usize> = m;

                        info!("Got MessagesIndexReceived: {:?} keep our index", m);
                    },
                    Ok(Telemetry::MessagesIndexSent(m)) => {
                      //  tel.index_sent = Some(m);

                        info!("Got MessagesIndexSent: {:?} keep our index", m);
                    },
                    Ok(Telemetry::MessagesReceived(m)) => {
                        info!("Got MessagesReceived: {:?} use index to store data", m);
                    },
                    Ok(Telemetry::MessagesSent(m)) => {
                        info!("Got MessagesSent: {:?} use index to store data", m);
                    },
                    Err(e) => {
                        error!("Unexpected error recv_async: {}",e);
                    }
                }
            }
        }
        Delay::new(Duration::from_millis(20)).await;
        if false {
            break Ok(());
        }
    }
}