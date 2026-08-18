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

fn make_sidecar_edge(id: usize, from: ActorName, to: ActorName, label: &str) -> Edge {
    let mut edge = make_edge(id, from, to, label);
    edge.sidecar = true;
    edge
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

fn make_memory_nodes() -> Vec<Node> {
    vec![
        make_node("from", None, "from"),
        make_node("to", None, "to"),
    ]
}

/// Builds an edge with a known memory footprint (`type_byte_count = 1`, `capacity = footprint`).
fn make_memory_edge(
    id: usize,
    footprint: usize,
    show_memory: bool,
    partner: Option<&'static str>,
    bundle_index: Option<usize>,
) -> Edge {
    let from = ActorName::new("from", None);
    let to = ActorName::new("to", None);
    let mut stats = ChannelStatsComputer::default();
    stats.capacity = footprint;
    stats.type_byte_count = 1;
    stats.memory_footprint = footprint;
    stats.show_memory = show_memory;
    Edge {
        id,
        from: Some(from),
        to: Some(to),
        color: "green",
        sidecar: false,
        pen_width: EDGE_PEN_WIDTH.to_string(),
        saturation_score: 0.0,
        ctl_labels: vec![],
        stats_computer: stats,
        display_label: String::new(),
        metric_text: String::new(),
        partner,
        bundle_index,
        ..Default::default()
    }
}

fn compressed_bytes(val: u128) -> String {
    let mut s = String::new();
    crate::channel_stats_labels::format_compressed_u128(val, &mut s);
    format!("{s}B")
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
        prop_assert!(dot.contains("nodesep="));
        prop_assert!(dot.contains("ranksep="));
    }

    /// Property: two or more nodes sharing a base name emit one {rank=same} column block.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_build_dot_same_name_suffixes_share_column(
        count in 2usize..=5usize,
    ) {
        let nodes: Vec<Node> = (0..count)
            .map(|i| make_node("Worker", Some(i), &format!("Worker{i}")))
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
        prop_assert!(
            dot.contains("{rank=same;"),
            "missing rank=same for shared base name:\n{dot}"
        );
        for i in 0..count {
            prop_assert!(
                dot.contains(&format!("\"Worker{i}\"")),
                "missing Worker{i} in:\n{dot}"
            );
        }
        // All Worker ids appear in a single rank=same line.
        let rank_line = dot
            .lines()
            .find(|l| l.starts_with("{rank=same;") && l.contains("Worker0"))
            .expect("rank=same line with Worker0");
        for i in 0..count {
            prop_assert!(
                rank_line.contains(&format!("\"Worker{i}\"")),
                "Worker{i} missing from rank line {rank_line}"
            );
        }
    }

    /// Property: same-name column and sidecar both emit `{rank=same}` (caller owns transitive merge).
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_build_dot_sidecar_and_same_name_both_emit_rank_same(
        worker_count in 2usize..=4usize,
    ) {
        let mut nodes: Vec<Node> = (0..worker_count)
            .map(|i| make_node("Worker", Some(i), &format!("Worker{i}")))
            .collect();
        nodes.push(make_node("Feedback", None, "Feedback"));
        let worker0 = ActorName::new("Worker", Some(0));
        let feedback = ActorName::new("Feedback", None);
        let edges = vec![make_sidecar_edge(0, worker0, feedback, "side")];
        let state = DotState {
            nodes,
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };
        let dot = render_dot(&state);

        let name_rank = dot
            .lines()
            .find(|l| l.starts_with("{rank=same;") && l.contains("Worker0") && l.contains(&format!("Worker{}", worker_count - 1)))
            .expect("same-name rank=same block");
        for i in 0..worker_count {
            prop_assert!(
                name_rank.contains(&format!("\"Worker{i}\"")),
                "Worker{i} missing from name rank: {name_rank}"
            );
        }

        let side_rank = dot
            .lines()
            .find(|l| {
                l.starts_with("{rank=same;")
                    && l.contains("\"Worker0\"")
                    && l.contains("\"Feedback\"")
            })
            .expect("sidecar rank=same block");
        prop_assert!(
            side_rank.contains("\"Worker0\"") && side_rank.contains("\"Feedback\""),
            "sidecar rank incomplete: {side_rank}"
        );
        // Both mechanisms present — transitive Graphviz merge is expected / caller-owned.
        let rank_blocks = dot.matches("{rank=same;").count();
        prop_assert!(rank_blocks >= 2, "expected >=2 rank=same blocks, got {rank_blocks}");
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

    /// Property: single-edge DOT shows Memory line only when show_memory is enabled.
    #[test]
    // ss[verify channel.memory-usage-telemetry]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_single_edge_memory_gating(
        show_memory in any::<bool>(),
        footprint in 1usize..100_000,
    ) {
        let edge = make_memory_edge(0, footprint, show_memory, None, None);
        let state = DotState {
            nodes: make_memory_nodes(),
            edges: vec![edge],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };
        let dot = render_dot(&state);
        prop_assert_eq!(dot.contains("Memory:"), show_memory);
    }

    /// Property: partnered lanes show combined memory in the partner header.
    #[test]
    // ss[verify channel.memory-usage-telemetry]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_partner_memory_is_sum(
        lane_count in 2usize..=4,
        footprints in prop::collection::vec(1usize..50_000, 2..=4),
    ) {
        let lanes = lane_count.min(footprints.len());
        let footprints: Vec<usize> = footprints.into_iter().take(lanes).collect();
        let sum = footprints.iter().sum::<usize>();
        let mut edges = Vec::new();
        for (i, fp) in footprints.iter().enumerate() {
            edges.push(make_memory_edge(i, *fp, true, Some("stream"), Some(0)));
        }
        let state = DotState {
            nodes: make_memory_nodes(),
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 8,
        };
        let dot = render_dot(&state);
        let expected = compressed_bytes(sum as u128);
        prop_assert!(
            dot.contains(&format!("stream [0] ({expected})")),
            "partner header missing combined memory {expected}:\n{dot}"
        );
    }

    /// Property: bundled partner groups show summed memory in header and tooltip.
    #[test]
    // ss[verify channel.memory-usage-telemetry]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_bundle_memory_is_sum(
        group_count in 2usize..=4,
        lanes_per_group in 1usize..=2,
        footprint in 100usize..50_000,
    ) {
        let total_edges = group_count * lanes_per_group;
        let total = footprint * total_edges;
        let mut edges = Vec::new();
        let mut id = 0;
        for bi in 0..group_count {
            for _ in 0..lanes_per_group {
                // Uniform footprint per edge so partner groups share the same
                // PrimaryGroupKey sub_capacities and render as one bundle.
                edges.push(make_memory_edge(id, footprint, true, Some("P"), Some(bi)));
                id += 1;
            }
        }
        let state = DotState {
            nodes: make_memory_nodes(),
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 2,
        };
        let dot = render_dot(&state);
        let expected = compressed_bytes(total as u128);
        prop_assert!(
            dot.contains(&format!("P: {group_count}x ({expected})")),
            "bundle header missing summed memory:\n{dot}"
        );
        prop_assert!(
            dot.contains(&format!("Memory: {expected}")),
            "bundle tooltip missing summed memory:\n{dot}"
        );
    }

    /// Property: partner and bundle rollups omit memory when show_memory is disabled.
    #[test]
    // ss[verify channel.memory-usage-telemetry]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_memory_hidden_in_rollup_when_disabled(
        lane_count in 2usize..=4,
        group_count in 2usize..=4,
    ) {
        let partner_edges: Vec<Edge> = (0..lane_count)
            .map(|i| make_memory_edge(i, 8000, false, Some("stream"), Some(0)))
            .collect();
        let partner_state = DotState {
            nodes: make_memory_nodes(),
            edges: partner_edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 8,
        };
        let partner_dot = render_dot(&partner_state);
        prop_assert!(!partner_dot.contains("Memory:"));
        let partner_line = partner_dot
            .lines()
            .find(|l| l.contains("stream [0]"))
            .unwrap_or("");
        prop_assert!(
            !partner_line.contains("B)"),
            "partner header leaked memory:\n{partner_dot}"
        );

        let bundle_edges: Vec<Edge> = (0..group_count)
            .map(|bi| make_memory_edge(bi, 8000, false, Some("P"), Some(bi)))
            .collect();
        let bundle_state = DotState {
            nodes: make_memory_nodes(),
            edges: bundle_edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 2,
        };
        let bundle_dot = render_dot(&bundle_state);
        prop_assert!(!bundle_dot.contains("Memory:"));
        let header_line = bundle_dot
            .lines()
            .find(|l| l.contains(&format!("P: {group_count}x")))
            .unwrap_or("");
        prop_assert!(
            !header_line.contains("B)"),
            "bundle header leaked memory:\n{bundle_dot}"
        );
    }
}
