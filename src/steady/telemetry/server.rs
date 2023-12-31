
//use tide::prelude::*; // Pulls in the json! macro.
//use tide_websockets::{Message, WebSocket};
use crate::steady::{SteadyMonitor, SteadyRx};
use crate::steady::telemetry::metrics_collector::DiagramData;
//use async_std::stream::StreamExt;
//use tide::{Endpoint, Request};
use bytes::{ BytesMut};
use bytes::buf::BufMut;
use log::info;

/*
// Define an async function to handle WebSocket connections.
async fn handle_ws(request: Request<()>) -> tide::Result<()> {
    WebSocket::new(|_req, mut stream| async move {
        while let Some(Ok(Message::Text(input))) = stream.next().await {
            // Echo the message back
            stream.send_string(input).await?;
        }
        Ok(())
    })
        .call(request)
        .await
}
*/
/*
async fn example() -> tide::Result<()> {
    let mut app = tide::new();

    // Add a route that listens for GET requests and upgrades to WebSocket.
    app.at("/ws").get(handle_ws);

    // Start the server
    app.listen("127.0.0.1:8080").await?;
    Ok(())
}
*/

pub(crate) async fn run(mut monitor: SteadyMonitor, rx: SteadyRx<DiagramData>) -> Result<(),()> {

    while rx.has_message() {
        match monitor.rx(&rx).await {
            Ok(DiagramData::Structure()) => {
                info!("Got DiagramData::Structure()");
            },
            Ok(DiagramData::Content()) => {
                info!("Got DiagramData::Content()");
            },
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

// nodes and edges should live between graphs
// we need to add custom labels to the nodes and edges

struct Node<'a> {
    id: & 'a String,
    color: & 'static str,
    pen_width: & 'static str,
    label: String,
    //TODO: nodes will require a count to unify replicas into a simpler graph..
}

struct Edge<'a> {
    from: & 'a String,
    to: & 'a String,
    color: & 'static str,
    pen_width: & 'static str,
    label: String,
}

fn build_dot(nodes: Vec<Node>, edges: Vec<Edge>, rankdir: &str, dot_graph: &mut BytesMut) {
    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice(rankdir.as_bytes());
    dot_graph.put_slice(b";\n");

    for node in nodes {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(node.id.as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(node.label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(node.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(node.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    for edge in edges {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(edge.from.as_bytes());
        dot_graph.put_slice(b"\" -> \"");
        dot_graph.put_slice(edge.to.as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(edge.label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(edge.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(edge.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    dot_graph.put_slice(b"}\n");
}



