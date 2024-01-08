
use crate::steady::{SteadyMonitor, SteadyRx, Node, Edge, build_dot};
use crate::steady::telemetry::metrics_collector::DiagramData;

use bytes::{ BytesMut};






pub(crate) async fn run(mut monitor: SteadyMonitor, rx: SteadyRx<DiagramData>) -> std::result::Result<(),()> {
//TODO: write to disk the dot graph and then the volume data in a separate file.

    //     monitor.init_stats(&[&rx], &[]); //TODO: this is not needed for this actor

    while rx.has_message() {
        match monitor.rx(&rx).await {
      //      Ok(DiagramData::Structure()) => {
     //           log::info!("Got DiagramData::Structure() to put in logs...");
      //      },
      //      Ok(DiagramData::Content()) => {
//info!("Got DiagramData::Content()");
      //      },
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
fn assemble_dot(nodes: Vec<Node>, edges: Vec<Edge>, top_down: bool, dot_graph: &mut BytesMut) -> BytesMut {
    dot_graph.clear(); // Clear the buffer for reuse
    let rankdir = if top_down { "TB" } else { "LR" };
    build_dot(nodes, edges, rankdir, dot_graph);

    // Use split_to to get the whole content as Bytes, without cloning
    dot_graph.split_to(dot_graph.len())
}


struct LocalState<'a> {
    nodes: Vec<Node<'a>>,
    edges: Vec<Edge<'a>>,
}




