use std::sync::Arc;
use bytes::{BufMut, BytesMut};
use log::error;

pub struct DotState {
    pub(crate) nodes: Vec<Node<>>, //position matches the node id
    pub(crate) edges: Vec<Edge<>>, //position matches the channel id
    pub seq: u64,
}

pub(crate) struct Node {
    pub id: usize,
    pub color: & 'static str,
    pub pen_width: & 'static str,
    pub display_label: String, //label may also have (n) for replicas
}

pub(crate) struct Edge<> {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) color: & 'static str,
    pub(crate) pen_width: & 'static str,
    pub(crate) display_label: String,
    pub(crate) id: usize,
    pub(crate) ctl_labels: Vec<&'static str>
}

pub fn build_dot(state: &DotState, rankdir: &str, dot_graph: &mut BytesMut) {
    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice(rankdir.as_bytes());
    dot_graph.put_slice(b";\n");
    dot_graph.put_slice(b"node [style=filled, fillcolor=white, fontcolor=black];\n");
    dot_graph.put_slice(b"edge [color=white, fontcolor=white];\n");
    dot_graph.put_slice(b"graph [bgcolor=black];\n");

    for node in &state.nodes {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(node.id.to_string().as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(node.display_label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(node.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(node.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    for edge in &state.edges {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(edge.from.to_string().as_bytes());
        dot_graph.put_slice(b"\" -> \"");
        dot_graph.put_slice(edge.to.to_string().as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(edge.display_label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(edge.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(edge.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    dot_graph.put_slice(b"}\n");
}



pub(crate) fn update_dot(mut graphvis_bytes: &mut BytesMut, rankdir: &str, mut dot_state: &mut DotState, msg: Result<crate::steady::telemetry::metrics_collector::DiagramData, String>) -> u64 {
    match msg {
        Ok(crate::steady::telemetry::metrics_collector::DiagramData::Node(seq, name, id, channels_in, channels_out)) => {
            refresh_structure(&mut dot_state, name, id, channels_in, channels_out);
            dot_state.seq = seq;
        },
        Ok(crate::steady::telemetry::metrics_collector::DiagramData::Edge(seq
                                                                          , total_take
                                                                          , total_send)) => {
            total_take.iter().enumerate().for_each(|(i,c)| {
               dot_state.edges[i].display_label = format!("out:{}",c);
            });
            dot_state.seq = seq;

            //TODO: update the chart with all the data in old example

            //NOTE: generate the graph and write it to disk
            graphvis_bytes.clear(); // Clear the buffer for reuse
            build_dot(dot_state, rankdir, &mut graphvis_bytes);

        }
        Err(msg) => {error!("Unexpected error on incomming message: {}",msg)}
    }
    dot_state.seq
}

pub fn refresh_structure(mut local_state: &mut DotState, name: &str
                         , id: usize
                         , channels_in: Arc<Vec<(usize, Vec<&'static str>)>>
                         , channels_out: Arc<Vec<(usize, Vec<&'static str>)>>) {
//rare but needed to ensure vector length
    if id.ge(&local_state.nodes.len()) {
        local_state.nodes.resize_with(id + 1, || {
            Node {
                id: usize::MAX,
                color: "grey",
                pen_width: "2",
                display_label: "".to_string(),
            }
        });
    }
    local_state.nodes[id].id = id;
    local_state.nodes[id].display_label = name.to_string();

    channels_in.iter()
        .for_each(|(cin,labels)| {
            if cin.ge(&local_state.edges.len()) {
                local_state.edges.resize_with(id + 1, || {
                    Edge {
                        from: usize::MAX,
                        to: usize::MAX,
                        color: "white",
                        pen_width: "3",
                        display_label: "".to_string(),//defined when the content arrives
                        id: usize::MAX,
                        ctl_labels: Vec::new(),
                    }
                });
            }
            let idx = *cin;
            local_state.edges[idx].id = idx;
            local_state.edges[idx].to = id;
            // Collect the labels that need to be added
            let labels_to_add: Vec<_> = labels.iter()
                .filter(|f| !local_state.edges[idx].ctl_labels.contains(f))
                .cloned()  // Clone the items to be added
                .collect();

// Now, append them to `ctl_labels`
            for label in labels_to_add {
                local_state.edges[idx].ctl_labels.push(label);
            }
        });

    channels_out.iter()
        .for_each(|(cout,labels)| {
            if cout.ge(&local_state.edges.len()) {
                local_state.edges.resize_with(id + 1, || {
                    Edge {
                        from: usize::MAX,
                        to: usize::MAX,
                        color: "white",
                        pen_width: "3",
                        display_label: "".to_string(),//defined when the content arrives
                        id: usize::MAX,
                        ctl_labels: Vec::new(),
                    }
                });
            }
            let idx = *cout;
            local_state.edges[idx].id = idx;
            local_state.edges[idx].from = id;
            // Collect the labels that need to be added
            let labels_to_add: Vec<_> = labels.iter()
                .filter(|f| !local_state.edges[idx].ctl_labels.contains(f))
                .cloned()  // Clone the items to be added
                .collect();

// Now, append them to `ctl_labels`
            for label in labels_to_add {
                local_state.edges[idx].ctl_labels.push(label);
            }
        });

}


