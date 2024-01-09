
use crate::steady::{SteadyMonitor, SteadyRx};
use crate::steady::telemetry::metrics_collector::DiagramData;
use crate::steady_util::steady_util;
//use async_std::stream::StreamExt;
use bytes::{ BytesMut};




pub(crate) async fn run(mut monitor: SteadyMonitor, rx: SteadyRx<DiagramData>) -> std::result::Result<(),()> {

    //     monitor.init_stats(&[&rx], &[]); //TODO: this is not needed for this actor

    //let mut app = tide::new();

    //TODO: this actor has two jobs. (this is implemented first)
    //   1. it needs to return dot graph data to the client upon get request
    //   2. it needs to return compressed history data to the client upon get request
    //      two parts, in memory and on disk.
    //

     // websocat is a command line tool for connecting to WebSockets servers
     // cargo install websocat
     // websocat ws://127.0.0.1:8080/ws
/*
    select! {
        _ = server_future.fuse() => {
            log::info!("Websocket server exited.");
        },
        _ = async_std::task::sleep(Duration::from_secs(10)) => {
            log::info!("Websocket server timed out.");
        },
        //we can read from the rx channel here.
        //TODO: we want the handle_ws outsourced to another
        //     actor child group so we can scale that up as needed
        //   this actor will be singular due to its need to hold the port.
     }
*/


    while rx.has_message() {
        match monitor.rx(&rx).await {
       //     Ok(DiagramData::Structure()) => {
      //          log::info!("Got DiagramData::Structure()");
       //     },
        //    Ok(DiagramData::Content()) => {
//info!("Got DiagramData::Content()");
        //    },
            _ => {}
        }
    }

    let local_state = LocalState {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut dot_graph = BytesMut::with_capacity(1024);
    assemble_dot(local_state.nodes, local_state.edges, true, &mut dot_graph);

    Ok(())
}

//   let mut dot_graph = BytesMut::with_capacity(1024); // Adjust capacity as needed
fn assemble_dot(nodes: Vec<steady_util::Node>, edges: Vec<steady_util::Edge>, top_down: bool, dot_graph: &mut BytesMut) -> BytesMut {
    dot_graph.clear(); // Clear the buffer for reuse
    let rankdir = if top_down { "TB" } else { "LR" };
    steady_util::build_dot(nodes, edges, rankdir, dot_graph);

    // Use split_to to get the whole content as Bytes, without cloning
    dot_graph.split_to(dot_graph.len())
}


struct LocalState<'a> {
    nodes: Vec<steady_util::Node<'a>>,
    edges: Vec<steady_util::Edge<'a>>,
}

// nodes and edges should live between graphs
// we need to add custom labels to the nodes and edges




