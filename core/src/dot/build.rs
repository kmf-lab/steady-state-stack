// ss[related telemetry.dot-export]
use std::collections::{BTreeMap, HashMap};
// ss[impl telemetry.dot-export]
use std::fmt::Write;

// ss[impl platform.ringbuf-pin]
use bytes::{BufMut, BytesMut};

// ss[impl telemetry.dot-export]
use crate::channel_stats::FilledVisualMode;
// ss[impl platform.ringbuf-pin]
use crate::dot_edge::Edge;
// ss[impl telemetry.dot-export]
use crate::ActorName;

// ss[impl platform.ringbuf-pin]
use super::colors::{actor_fillcolor_hex_into, color_to_rgb};
// ss[impl telemetry.dot-export]
use super::escape::escape_node_tooltip_text;
// ss[impl telemetry.dot-export]
use super::format::{
    append_channel_fill_tooltip, format_avg_fill_rollup_line_into, format_lane_color_histogram_into,
    hex_color_average_into, mean_avg_fill_percent,
};
// ss[impl platform.ringbuf-pin]
use super::frames::DotGraphFrames;
// ss[impl telemetry.dot-export]
use super::keys::{PartnerKey, PrimaryGroupKey};
// ss[impl telemetry.dot-export]
use super::partnered::PartneredEdge;
// ss[impl platform.ringbuf-pin]
use super::render::render_edge_internal;
// ss[impl telemetry.dot-export]
use super::{
    BUNDLE_PEN_WIDTH, DOT_NODESEP, DOT_RANKSEP, DotState, MAX_INLINE_AVG_FILL_LANES,
    PARTNER_BUNDLE_PEN_WIDTH,
};

/// Emit `{rank=same}` for each base actor name that has two or more instances.
///
/// Under `rankdir=LR`, Graphviz places same-rank nodes in one column. Actors created with
/// `with_name_and_suffix("Worker", i)` share a column without a new builder API.
///
/// `connects_sidecar()` also emits pairwise `{rank=same}` (same mechanism, intentional).
/// Graphviz ranks are **transitive**: if a sidecar edge ties one sibling into another actor,
/// that partner can join the shared-name column. Using code owns naming and which edges are
/// sidecar so the diagram stays readable — Steady State will not exclude sidecar endpoints
/// from same-name groups.
// ss[impl platform.ringbuf-pin]
fn emit_same_name_rank_columns(dot_graph: &mut BytesMut, state: &DotState) {
    let mut by_name: BTreeMap<&'static str, Vec<ActorName>> = BTreeMap::new();
    for node in state.nodes.iter().filter(|n| n.id.is_some()) {
        if let Some(id) = node.id {
            by_name.entry(id.name).or_default().push(id);
        }
    }
    for (_name, mut members) in by_name {
        if members.len() < 2 {
            continue;
        }
        // Stable order: Some(suffix) ascending, then suffix-less names.
        members.sort_by(|a, b| match (a.suffix, b.suffix) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        // Deduplicate identical ActorName entries if the node list ever repeats an id.
        members.dedup();
        if members.len() < 2 {
            continue;
        }

        dot_graph.put_slice(b"{rank=same;");
        for m in &members {
            dot_graph.put_slice(b" \"");
            dot_graph.put_slice(m.name.as_bytes());
            if let Some(s) = m.suffix {
                dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
            }
            dot_graph.put_slice(b"\"");
        }
        dot_graph.put_slice(b"}\n");
    }
}

/// Builds the DOT graph from the current state.
///
/// # Arguments
///
/// * `state` - THE current metric state.
/// * `frames` - Working buffers including the DOT output (`active_graph`).
// ss[impl platform.ringbuf-pin]
pub(crate) fn build_dot(state: &DotState, frames: &mut DotGraphFrames) {
    frames.active_graph.clear(); // Clear the buffer for reuse
    let dot_graph = &mut frames.active_graph;

    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice("LR".as_bytes());
    dot_graph.put_slice(b";\n");

    // Graphviz `dot` has no gravity; these two attributes are the layout tightness.
    dot_graph.put_slice(b"graph [nodesep=");
    dot_graph.put_slice(DOT_NODESEP.as_bytes());
    dot_graph.put_slice(b", ranksep=");
    dot_graph.put_slice(DOT_RANKSEP.as_bytes());
    dot_graph.put_slice(b"];\n");
    dot_graph.put_slice(b"node [margin=0.1];\n"); // Gap around text inside the circle

    dot_graph.put_slice(b"node [style=filled, fillcolor=white, fontcolor=black, fontname=Helvetica, fontsize=14];\n");
    dot_graph.put_slice(b"edge [color=white, fontcolor=white, fontname=Helvetica, fontsize=12];\n");
    dot_graph.put_slice(b"graph [bgcolor=black];\n");

    state
        .nodes
        .iter()
        .filter(|n| {
            // Only fully defined nodes, some may be in the process of being defined
            n.id.is_some()
        })
        .for_each(|node| {
            dot_graph.put_slice(b"\"");

            if let Some(f) = node.id {
                dot_graph.put_slice(f.name.as_bytes());
                if let Some(s) = f.suffix {
                    dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
                }
            } else {
                dot_graph.put_slice(b"No Name");
            }

            dot_graph.put_slice(b"\" [label=\"");
            dot_graph.put_slice(node.display_label.as_bytes());
            if !node.tooltip.is_empty() {
                dot_graph.put_slice(b"\", tooltip=\"");
                escape_node_tooltip_text(&mut frames.dot_scratch, &node.tooltip);
                dot_graph.put_slice(frames.dot_scratch.as_bytes());
            }
            if !node.color.is_empty() {
                dot_graph.put_slice(b"\", color=\"");
                dot_graph.put_slice(node.color.as_bytes());
            }
            actor_fillcolor_hex_into(
                &mut frames.hex_line,
                if node.color.is_empty() {
                    ""
                } else {
                    node.color
                },
            );
            dot_graph.put_slice(b"\", fillcolor=\"");
            dot_graph.put_slice(frames.hex_line.as_bytes());
            dot_graph.put_slice(b"\", penwidth=");
            dot_graph.put_slice(node.pen_width.as_bytes());
            dot_graph.put_slice(b" ");

            if let Some(remote) = &node.remote_details {
                dot_graph.put_slice(b"/* remote_ips='");
                dot_graph.put_slice(remote.ips.as_bytes());
                dot_graph.put_slice(b"', match_on='");
                dot_graph.put_slice(remote.match_on.as_bytes());
                dot_graph.put_slice(b"', direction='");
                dot_graph.put_slice(remote.direction.as_bytes());
                dot_graph.put_slice(b"', tech='");
                dot_graph.put_slice(remote.tech.as_bytes());
                dot_graph.put_slice(b"' */");
            };
            dot_graph.put_slice(b"];\n");
        });

    // Same base name + distinct suffixes → one LR column (tighter packing).
    emit_same_name_rank_columns(dot_graph, state);

    // Stage 1: Partnering
    let mut partner_groups: BTreeMap<PartnerKey, Vec<&Edge>> = BTreeMap::new();
    for edge in state
        .edges
        .iter()
        .filter(|e| e.id != usize::MAX && e.from.is_some() && e.to.is_some())
    {
        let key = PartnerKey {
            from: edge.from.map(|n| (n.name, n.suffix)),
            to: edge.to.map(|n| (n.name, n.suffix)),
            partner: edge.partner,
            bundle_index: edge.bundle_index,
            edge_id: if edge.partner.is_none() {
                Some(edge.id)
            } else {
                None
            },
        };
        partner_groups.entry(key).or_default().push(edge);
    }

    let mut partnered_edges = Vec::new();
    for (_, mut edges) in partner_groups {
        // S-Tier: Sort edges by type name to ensure index-aligned vectors are stable across the bundle
        edges.sort_by_key(|e| e.stats_computer.show_type.unwrap_or(""));

        let first = edges[0];
        let is_partnered = first.partner.is_some();
        let mut lane_rgbs = Vec::new();
        let mut type_list = Vec::new();
        let mut ctl_labels = Vec::new();
        let mut ids = Vec::new();
        let mut tooltip = String::new();

        // Add Window info to the top of the tooltip if active
        if !first.stats_computer.time_label.is_empty() {
            let _ = write!(tooltip, "Window: {}\n", first.stats_computer.time_label);
        }

        let mut sum_saturation = 0.0;
        let mut sub_capacities = Vec::with_capacity(edges.len());
        let mut sub_totals = Vec::with_capacity(edges.len());
        let mut sum_total_consumed = 0u128;
        let mut memory_footprint = 0;
        let mut show_memory = false;
        let mut lane_colors = Vec::with_capacity(edges.len());
        let mut avg_fill_per_lane = Vec::with_capacity(edges.len());

        let is_large_bundle = edges.len() > MAX_INLINE_AVG_FILL_LANES;

        for e in edges.iter() {
            lane_rgbs.push(color_to_rgb(e.color));
            lane_colors.push(e.color);
            avg_fill_per_lane.push(e.stats_computer.avg_filled_whole_percent());

            let short_type = e.stats_computer.show_type.unwrap_or("");
            if !short_type.is_empty() {
                type_list.push(short_type);
            }
                sub_capacities.push(e.stats_computer.capacity);
            sub_totals.push(e.stats_computer.total_consumed);
            memory_footprint += e.stats_computer.memory_footprint;
            show_memory |= e.stats_computer.show_memory;

            if !is_large_bundle {
                let _ = write!(
                    tooltip,
                    "CH#{}: {}\n",
                    e.id,
                    if short_type.is_empty() {
                        "Data"
                    } else {
                        short_type
                    }
                );
                tooltip.push_str(" Capacity: ");
                crate::channel_stats_labels::format_compressed_u128(
                    e.stats_computer.capacity as u128,
                    &mut tooltip,
                );
                // Per-lane memory footprint when the channel opted into memory display
                if e.stats_computer.show_memory {
                    tooltip.push_str("\n Memory: ");
                    crate::channel_stats_labels::format_compressed_u128(
                        e.stats_computer.memory_footprint as u128,
                        &mut tooltip,
                    );
                    tooltip.push('B');
                }
                // FIX: Show Total (cumulative) on tooltip to match edge label
                tooltip.push_str("\n Total: ");
                crate::channel_stats_labels::format_compressed_u128(
                    e.stats_computer.total_consumed,
                    &mut tooltip,
                );
                append_channel_fill_tooltip(&mut tooltip, &e.stats_computer, e.saturation_score);
            }

            ids.push(e.id);
            sum_saturation += e.saturation_score;
            sum_total_consumed += e.stats_computer.total_consumed;

            for &l in &e.ctl_labels {
                if !ctl_labels.contains(&l) {
                    ctl_labels.push(l);
                }
            }
        }

        // Only show avg fill if at least one lane has a non-zero value (skip idle/cold channels).
        let show_avg_filled_any = avg_fill_per_lane.iter().any(|o| o.map_or(false, |v| v > 0));

        if !is_large_bundle && !lane_colors.is_empty() {
            format_lane_color_histogram_into(
                &mut frames.lane_color_counts,
                &mut frames.dot_scratch,
                &lane_colors,
            );
            let _ = write!(tooltip, "{}\n", frames.dot_scratch);
        }

        if is_large_bundle {
            let _ = write!(tooltip, "Summary: {} channels\n", edges.len());

            // Total volume across all channels
            let _ = write!(tooltip, "Total: ");
            crate::channel_stats_labels::format_compressed_u128(
                sum_total_consumed,
                &mut tooltip,
            );
            tooltip.push('\n');

            // Capacity range
            let cap_min = sub_capacities.iter().copied().min().unwrap_or(0);
            let cap_max = sub_capacities.iter().copied().max().unwrap_or(0);
            if cap_min == cap_max {
                let _ = write!(tooltip, "Capacity: ");
                crate::channel_stats_labels::format_compressed_u128(
                    cap_min as u128,
                    &mut tooltip,
                );
                tooltip.push('\n');
            } else {
                let _ = write!(tooltip, "Capacity: ");
                crate::channel_stats_labels::format_compressed_u128(
                    cap_min as u128,
                    &mut tooltip,
                );
                tooltip.push_str("–");
                crate::channel_stats_labels::format_compressed_u128(
                    cap_max as u128,
                    &mut tooltip,
                );
                tooltip.push('\n');
            }

            // Mean avg fill
            if let Some(m) = mean_avg_fill_percent(avg_fill_per_lane.iter()) {
                let _ = write!(tooltip, "Avg fill: {}% (mean)\n", m);
            }

            // Mean saturation
            let mean_sat = (sum_saturation / edges.len() as f64 * 100.0) as usize;
            let _ = write!(tooltip, "Saturation: {}% (mean)\n", mean_sat);

            // Memory footprint
            if show_memory {
                tooltip.push_str("Memory: ");
                crate::channel_stats_labels::format_compressed_u128(
                    memory_footprint as u128,
                    &mut tooltip,
                );
                tooltip.push_str("B\n");
            }

            // Lane color histogram
            if !lane_colors.is_empty() {
                format_lane_color_histogram_into(
                    &mut frames.lane_color_counts,
                    &mut frames.dot_scratch,
                    &lane_colors,
                );
                let _ = write!(tooltip, "{}\n", frames.dot_scratch);
            }
        }

        let combined_type = type_list.join("/");

        // Multi-lane avg fill: rebuild first lane's label without "Avg filled" line, then append comma rollup.
        // When all lanes are idle (0% fill / no sample), suppress the individual avg fill line too.
        let mut summary_label = if edges.len() > 1 {
            let mut body = String::new();
            let mut dummy_metric = String::new();
            first.stats_computer.append_visual_metric_lines(
                &mut body,
                &mut dummy_metric,
                FilledVisualMode::SuppressAvgOnly,
            );
            if show_avg_filled_any {
                format_avg_fill_rollup_line_into(&mut frames.dot_scratch, &edges);
                body.push_str(frames.dot_scratch.trim_end_matches('\n'));
            }
            body
        } else {
            first.display_label.clone()
        };

        // CRITICAL: For partnered edges, we must preserve the display_label which contains the
        // rate/filled metrics computed by ChannelStatsComputer. The partner info is prepended
        // to the existing label rather than replacing it entirely.
        if is_partnered {
            // Prepend partner identifier to the existing label, don't replace it
            let partner_header = format!(
                "{} [{}]",
                first.partner.unwrap(),
                first.bundle_index.unwrap_or(0)
            );

            // Build partner info line with memory if needed
            let mut partner_info = partner_header;
            if show_memory {
                partner_info.push_str(" (");
                crate::channel_stats_labels::format_compressed_u128(
                    memory_footprint as u128,
                    &mut partner_info,
                );
                partner_info.push_str("B)");
            }

            // Combine: Partner info first, then original label (which contains rate/fill from ChannelStatsComputer)
            // This ensures avg_rate and other dynamic metrics are preserved on the label
            summary_label = format!("{}\n{}", partner_info, summary_label.trim_end_matches('\n'));
        } else if edges.len() == 1 && show_memory {
            // Single, non-partnered channel opted into memory display: append its
            // reserved buffer footprint to the edge label, matching the partnered
            // header format `(N B)`.
            let mut mem_info = String::from("Memory: ");
            crate::channel_stats_labels::format_compressed_u128(
                memory_footprint as u128,
                &mut mem_info,
            );
            mem_info.push('B');
            summary_label = format!("{}\n{}", summary_label.trim_end_matches('\n'), mem_info);
        }

        // FIX: Always show the total(s) on the edge label itself, not just in the tooltip.
        // For bundled edges that will be rendered individually (len < bundle_floor_size),
        // show all totals separated by commas. For single edges, show the total directly.
        // For large bundles (len >= bundle_floor_size), totals are handled in the bundle rendering section below.
        if first.stats_computer.show_total {
            let mut total_label = String::new();
            if edges.len() == 1 {
                // Single edge: show total directly
                total_label.push_str("Total: ");
                crate::channel_stats_labels::format_compressed_u128(
                    first.stats_computer.total_consumed,
                    &mut total_label,
                );
            } else if edges.len() < state.bundle_floor_size {
                // Bundled edges (but not large enough to be rendered as bundle): show all totals separated by commas
                total_label.push_str("Totals: ");
                for (i, total) in sub_totals.iter().enumerate() {
                    if i > 0 {
                        total_label.push_str(", ");
                    }
                    crate::channel_stats_labels::format_compressed_u128(*total, &mut total_label);
                }
            }
            // For large bundles (len >= bundle_floor_size), the totals are handled in the bundle rendering section below
            if !total_label.is_empty() {
                summary_label = format!("{}{}\n", summary_label.trim_end_matches('\n'), total_label);
            }
        }

        partnered_edges.push(PartneredEdge {
            from: first.from,
            to: first.to,
            lane_rgbs,
            lane_colors,
            avg_fill_per_lane,
            show_avg_filled_any,
            summary_label,
            combined_type,
            partner_name: first.partner,
            sub_capacities,
            sidecar: first.sidecar,
            saturation_score: sum_saturation / edges.len() as f64,
            tooltip,
            sub_totals,
            ids,
            ctl_labels,
            pen_width: if is_partnered {
                PARTNER_BUNDLE_PEN_WIDTH.to_string()
            } else {
                first.pen_width.clone()
            },
            memory_footprint,
            show_memory,
            show_total: first.stats_computer.show_total,
        });
    }

    // Stage 2: Aggregation
    let mut primary_groups: HashMap<PrimaryGroupKey, Vec<PartneredEdge>> = HashMap::new();
    for pe in partnered_edges {
        let key = PrimaryGroupKey {
            from_name: pe.from.map(|f| f.name),
            from_suffix: pe.from.and_then(|f| f.suffix),
            to_name: pe.to.map(|f| f.name),
            to_suffix: pe.to.and_then(|f| f.suffix),
            sub_capacities: pe.sub_capacities.clone(),
            type_name: pe.combined_type.clone(),
            sidecar: pe.sidecar,
            partner: pe.partner_name,
        };
        primary_groups.entry(key).or_default().push(pe);
    }

    // Sort keys to ensure deterministic DOT output
    let mut sorted_primary: Vec<_> = primary_groups.keys().collect();
    sorted_primary.sort();

    for p_key in sorted_primary {
        let edges = &primary_groups[p_key];

        if edges.len() < state.bundle_floor_size {
            for pe in edges {
                hex_color_average_into(&mut frames.hex_line, &pe.lane_rgbs);

                render_edge_internal(
                    dot_graph,
                    p_key.from_name.unwrap_or("unknown"),
                    p_key.from_suffix,
                    p_key.to_name.unwrap_or("unknown"),
                    p_key.to_suffix,
                    &pe.summary_label,
                    &frames.hex_line,
                    &pe.pen_width,
                    "",
                    p_key.sidecar,
                    "",
                    "",
                    &pe.tooltip,
                    &mut frames.dot_scratch,
                );
            }
        } else {
            // Render as Bundle
            let n = edges.len();
            let total_channels: usize = edges.iter().map(|e| e.ids.len()).sum();
            let sum_traffic: f64 = edges
                .iter()
                .map(|e| e.saturation_score * e.sub_capacities.iter().sum::<usize>() as f64)
                .sum();
            let bundle_capacity = (n as f64) * p_key.sub_capacities.iter().sum::<usize>() as f64;
            let _bundle_util = if bundle_capacity > 0.0 {
                (sum_traffic / bundle_capacity).clamp(0.0, 1.0)
            } else {
                0.0
            };

            // S-Tier: Element-wise summation of index-aligned totals
            let mut bundle_totals = vec![0u128; edges[0].sub_totals.len()];
            let mut total_memory = 0usize;
            let mut show_mem = false;
            let mut bundle_memory_line = String::new();

            for pe in edges {
                for (i, val) in pe.sub_totals.iter().enumerate() {
                    bundle_totals[i] += val;
                }
                total_memory += pe.memory_footprint;
                show_mem |= pe.show_memory;
            }

            let mut all_labels: Vec<&'static str> = edges
                .iter()
                .flat_map(|e| e.ctl_labels.iter().cloned())
                .collect();
            all_labels.sort();
            all_labels.dedup();

            let label_prefix = p_key.partner.unwrap_or("Bundle");
            let mut header = format!("{}: {}x", label_prefix, n);

            if show_mem {
                header.push_str(" (");
                crate::channel_stats_labels::format_compressed_u128(
                    total_memory as u128,
                    &mut header,
                );
                header.push_str("B)");
                // Mirror the bundle memory total in the tooltip so users can see it on hover
                bundle_memory_line = format!(
                    "Memory: {}B",
                    {
                        let mut s = String::new();
                        crate::channel_stats_labels::format_compressed_u128(total_memory as u128, &mut s);
                        s
                    }
                );
            }
            // FIX: Show comma-separated totals for each partner type in the bundle, not a single aggregated total
            if edges[0].show_total {
                header.push_str("\nTotals: ");
                for (i, total) in bundle_totals.iter().enumerate() {
                    if i > 0 {
                        header.push_str(", ");
                    }
                    crate::channel_stats_labels::format_compressed_u128(*total, &mut header);
                }
            }
            if edges.iter().any(|pe| pe.show_avg_filled_any) {
                frames.dot_scratch.clear();
                if total_channels > MAX_INLINE_AVG_FILL_LANES {
                    if let Some(m) = mean_avg_fill_percent(
                        edges
                            .iter()
                            .flat_map(|pe| pe.avg_fill_per_lane.iter()),
                    ) {
                        let _ = write!(
                            frames.dot_scratch,
                            "{}% (mean, {} ch)",
                            m, total_channels
                        );
                    }
                } else {
                    let mut started = false;
                    for o in edges.iter().flat_map(|pe| pe.avg_fill_per_lane.iter()) {
                        if let Some(n) = o {
                            if *n == 0 {
                                continue; // skip idle/cold lanes (0% fill)
                            }
                            if !started {
                                started = true;
                            } else {
                                frames.dot_scratch.push_str(", ");
                            }
                            let _ = write!(frames.dot_scratch, "{}%", n);
                        }
                    }
                }
                if !frames.dot_scratch.is_empty() {
                    header.push_str("\nAvg fill: ");
                    header.push_str(&frames.dot_scratch);
                }
            }
            if !p_key.type_name.is_empty() {
                header.push_str("\n");
                header.push_str(&p_key.type_name);
            }
            all_labels.iter().for_each(|l| {
                header.push(' ');
                header.push_str(l);
            });

            let mut bundle_tooltip = format!("Bundle ({} chans in {} groups):", total_channels, n);
            // Add Window info to the top of the bundle tooltip
            if !edges[0].tooltip.is_empty() && edges[0].tooltip.starts_with("Window:") {
                if let Some(first_line) = edges[0].tooltip.split("\\n").next() {
                    bundle_tooltip.push_str("\\n");
                    bundle_tooltip.push_str(first_line);
                }
            }
            // Surface the summed memory footprint in the tooltip as well as the header
            if !bundle_memory_line.is_empty() {
                bundle_tooltip.push_str("\\n");
                bundle_tooltip.push_str(&bundle_memory_line);
            }

            if total_channels > MAX_INLINE_AVG_FILL_LANES {
                // Large bundle tooltip - show summary, but no total volume or avg saturation
                let _ = write!(bundle_tooltip, "\\nSummary: {} channels", total_channels);
            } else {
                if p_key.sub_capacities.len() > 1 {
                    bundle_tooltip.push_str("\\n Capacities: (");
                    for (i, cap) in p_key.sub_capacities.iter().enumerate() {
                        if i > 0 {
                            bundle_tooltip.push_str(", ");
                        }
                        crate::channel_stats_labels::format_compressed_u128(
                            *cap as u128,
                            &mut bundle_tooltip,
                        );
                    }
                    bundle_tooltip.push_str(")");
                }
                for e in edges.iter() {
                    // Skip the Window line if it was already added to the bundle header
                    let entry_tooltip = if e.tooltip.starts_with("Window:") {
                        e.tooltip.splitn(2, "\\n").nth(1).unwrap_or(&e.tooltip)
                    } else {
                        &e.tooltip
                    };
                    bundle_tooltip.push_str("\\n");
                    bundle_tooltip.push_str(entry_tooltip);
                }
            }

            let flat_lane_colors: Vec<&'static str> = edges
                .iter()
                .flat_map(|pe| pe.lane_colors.iter().copied())
                .collect();
            if !flat_lane_colors.is_empty() {
                format_lane_color_histogram_into(
                    &mut frames.lane_color_counts,
                    &mut frames.dot_scratch,
                    &flat_lane_colors,
                );
                let _ = write!(bundle_tooltip, "\\n{}", frames.dot_scratch);
            }

            let is_partnered = p_key.partner.is_some();
            let pen_width = if is_partnered {
                PARTNER_BUNDLE_PEN_WIDTH
            } else {
                BUNDLE_PEN_WIDTH
            };

            let all_rgbs: Vec<(u32, u32, u32)> = edges
                .iter()
                .flat_map(|pe| pe.lane_rgbs.iter().cloned())
                .collect();
            hex_color_average_into(&mut frames.hex_line, &all_rgbs);

            render_edge_internal(
                dot_graph,
                p_key.from_name.unwrap_or("unknown"),
                p_key.from_suffix,
                p_key.to_name.unwrap_or("unknown"),
                p_key.to_suffix,
                &header,
                &frames.hex_line,
                pen_width,
                ", style=\"bold,dashed\"",
                p_key.sidecar,
                "",
                "",
                &bundle_tooltip,
                &mut frames.dot_scratch,
            );
        }
    }
    dot_graph.put_slice(b"}\n");
}
