use std::sync::Arc;
use async_std::sync::Mutex;
use crate::steady::{SteadyMonitor, SteadyRx};
use crate::steady::telemetry::metrics_collector::DiagramData;
use crate::steady_util::steady_util;
use bytes::{ BytesMut};






pub(crate) async fn run(monitor: SteadyMonitor, rx: Arc<Mutex<SteadyRx<DiagramData>>>) -> std::result::Result<(),()> {
//TODO: write to disk the dot graph and then the volume data in a separate file.

    //     monitor.init_stats(&[&rx], &[]); //TODO: this is not needed for this actor
/*
    while rx.has_message() {
        match monitor.rx(& mut rx).await {
      //      Ok(DiagramData::Structure()) => {
     //           log::info!("Got DiagramData::Structure() to put in logs...");
      //      },
      //      Ok(DiagramData::Content()) => {
//info!("Got DiagramData::Content()");
      //      },
            _ => {}
        }
    }
*/
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




