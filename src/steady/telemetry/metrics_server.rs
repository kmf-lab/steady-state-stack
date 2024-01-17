use std::ops::DerefMut;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use async_std::sync::Mutex;
use crate::steady::SteadyRx;
use crate::steady::telemetry::metrics_collector::DiagramData;
use crate::steady::dot::*;
// Provides the .fuse() method
use bytes::BytesMut;
use crate::steady::monitor::SteadyMonitor;

struct AppState {
    dot_graph: Arc<AtomicUsize>, // Stores a pointer to the current DOT graph
}

// websocat is a command line tool for connecting to WebSockets servers
// cargo install websocat
// websocat ws://127.0.0.1:8080/ws

//TODO:  using ntex server
//       poll for latest dot file http/2
//       stream send/take data on websocket
//       poll for history file (gap?) (send notice of flush)



pub(crate) async fn run(monitor: SteadyMonitor, rx: Arc<Mutex<SteadyRx<DiagramData>>>) -> std::result::Result<(),()> {

    let mut rx_guard = rx.lock().await;
    let rx = rx_guard.deref_mut();

    //     monitor.init_stats(&[&rx], &[]); //TODO: this is not needed for this telemetry
    let mut dot_state = DotState {
        seq: u128::MAX,
        nodes: Vec::new(), //position is the telemetry id
        edges: Vec::new(), //position is the channel id
    };

    let top_down = true;
    let rankdir = if top_down { "TB" } else { "LR" };
    let mut dot_graph = BytesMut::new();

    loop {
       // select! {
          // _ = server_future => {
            //    log::info!("Web server exited.");
            //},
            let msg = rx.take_async().await; // =>
         {
                   let seq = update_dot(&mut dot_graph, rankdir, &mut dot_state, msg);
            }
        //,
       // }
    }


    Ok(())
}

