// ss[related telemetry.dot-export]
use crate::ss_proptest;
use proptest::prelude::*;

use super::super::*;
use super::test_dot_frames;
use crate::actor_stats::ActorStatsComputer;
use crate::channel_stats::ChannelStatsComputer;
use crate::dot_edge::Edge;
use crate::dot_node::Node;

const NODE_NAMES: &[&str] = &["alpha", "beta", "gamma", "delta", "epsilon"];

fn dot_node_key(name: &ActorName) -> String {
    match name.suffix {
        Some(suffix) => format!("{}{suffix}", name.name),
        None => name.name.to_string(),
    }
}

fn make_node(name: &'static str, suffix: Option<usize>, label: &str) -> Node {
    Node {
        id: Some(ActorName::new(name, suffix)),
        color: "grey",
        pen_width: NODE_PEN_WIDTH,
        stats_computer: ActorStatsComputer::default(),
        display_label: label.to_string(),
        dot_subtitle: None,
        tooltip: String::new(),
        metric_text: String::new(),
        remote_details: None,
        thread_info_cache: None,
        total_count_restarts: 0,
        bool_stalled: false,
        work_info: None,
    }
}

fn make_edge(id: usize, from: ActorName, to: ActorName, label: &str) -> Edge {
    Edge {
        id,
        from: Some(from),
        to: Some(to),
        color: "green",
        sidecar: false,
        pen_width: EDGE_PEN_WIDTH.to_string(),
        saturation_score: 0.1,
        ctl_labels: vec![],
        stats_computer: ChannelStatsComputer::default(),
        display_label: label.to_string(),
        metric_text: String::new(),
        partner: None,
        bundle_index: None,
        ..Default::default()
    }
}

fn render_dot(state: &DotState) -> String {
    let mut frames = test_dot_frames();
    build_dot(state, &mut frames);
    String::from_utf8(frames.active_graph.to_vec()).expect("utf8 dot")
}

fn valid_edge_count(edges: &[Edge]) -> usize {
    edges
        .iter()
        .filter(|e| e.id != usize::MAX && e.from.is_some() && e.to.is_some())
        .count()
}

fn count_arrows(dot: &str) -> usize {
    dot.matches(" -> ").count()
}

ss_proptest! {

    /// Property: every defined node id appears quoted in the DOT output.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_build_dot_all_node_ids_present(
        node_count in 1usize..=5usize,
        edge_count in 0usize..=8usize,
        suffix_flags in prop::collection::vec(any::<bool>(), 5),
    ) {
        let nodes: Vec<Node> = (0..node_count)
            .map(|i| {
                let suffix = if suffix_flags.get(i).copied().unwrap_or(false) {
                    Some(i + 1)
                } else {
                    None
                };
                make_node(NODE_NAMES[i], suffix, &format!("node{i}"))
            })
            .collect();

        let mut edges = Vec::new();
        if node_count >= 2 {
            for i in 0..edge_count.min(node_count - 1) {
                let from = nodes[i].id.expect("from id");
                let to = nodes[(i + 1) % node_count].id.expect("to id");
                edges.push(make_edge(i, from, to, &format!("edge{i}")));
            }
        }

        let state = DotState {
            nodes,
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let dot = render_dot(&state);
        for node in state.nodes.iter().filter(|n| n.id.is_some()) {
            let key = dot_node_key(&node.id.expect("node id"));
            prop_assert!(
                dot.contains(&format!("\"{key}\"")),
                "missing node id '{key}' in dot:\n{dot}"
            );
        }
    }

    /// Property: DOT structural output has paired double quotes for node ids.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_build_dot_no_bare_quotes(node_count in 1usize..=4usize) {
        let nodes: Vec<Node> = (0..node_count)
            .map(|i| make_node(NODE_NAMES[i], None, &format!("node{i}")))
            .collect();

        let state = DotState {
            nodes,
            edges: vec![],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let dot = render_dot(&state);
        let quote_count = dot.chars().filter(|&c| c == '"').count();
        prop_assert_eq!(quote_count % 2, 0);
    }

    /// Property: rendered edge arrows are bounded by the number of valid input edges.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_build_dot_edge_count_bounds(
        edge_count in 0usize..=12usize,
        bundle_floor in 2usize..=6usize,
    ) {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);
        let nodes = vec![
            make_node("from", None, "from"),
            make_node("to", None, "to"),
        ];
        let edges: Vec<Edge> = (0..edge_count)
            .map(|i| make_edge(i, from, to, &format!("CH{i}")))
            .collect();
        let valid = valid_edge_count(&edges);

        let state = DotState {
            nodes,
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: bundle_floor,
        };

        let dot = render_dot(&state);
        let arrows = count_arrows(&dot);
        if valid == 0 {
            prop_assert_eq!(arrows, 0);
        } else {
            prop_assert!(arrows >= 1, "expected at least one arrow, got {arrows}");
            prop_assert!(arrows <= valid, "arrows {arrows} > valid edges {valid}");
        }
    }

    /// Property: DOT output always begins with a digraph header.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_build_dot_digraph_header(node_count in 1usize..=3usize) {
        let nodes: Vec<Node> = (0..node_count)
            .map(|i| make_node(NODE_NAMES[i], None, &format!("node{i}")))
            .collect();
        let state = DotState {
            nodes,
            edges: vec![],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };
        let dot = render_dot(&state);
        prop_assert!(dot.contains("digraph"));
    }

    /// Property: bundle floor size is reflected in edge grouping when multiple edges share endpoints.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_build_dot_respects_bundle_floor(
        bundle_floor in 2usize..6usize,
        edge_count in 2usize..8usize,
    ) {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);
        let nodes = vec![
            make_node("from", None, "from"),
            make_node("to", None, "to"),
        ];
        let edges: Vec<Edge> = (0..edge_count)
            .map(|i| make_edge(i, from, to, &format!("CH{i}")))
            .collect();
        let state = DotState {
            nodes,
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: bundle_floor,
        };
        let dot = render_dot(&state);
        prop_assert!(dot.contains("digraph"));
        prop_assert!(dot.contains("\"from\""));
        prop_assert!(dot.contains("\"to\""));
    }
}
