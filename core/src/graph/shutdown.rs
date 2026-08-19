// ss[related graph.block-until-stopped]
use super::deps::*;
// ss[related philosophy.structural-hierarchy]
use super::liveliness::GraphLiveliness;
// ss[related philosophy.structural-hierarchy]
use super::state::GraphLivelinessState;
// ss[related graph.block-until-stopped]
use log::{debug, warn};

/// Minimum shutdown wait used by [`Graph::block_until_stopped`], derived from telemetry cadence.
// ss[related graph.for-testing]
pub(crate) fn effective_block_until_stopped_timeout(
    clean_shutdown_timeout: Duration,
    telemetry_production_rate_ms: u64,
) -> Duration {
    clean_shutdown_timeout.max(Duration::from_millis(3 * telemetry_production_rate_ms))
}

/// Monitors the shutdown process until completion or timeout.
// ss[related graph.for-testing]
pub(crate) fn watch_shutdown(
    timeout: Duration,
    now: Instant,
    rs: Arc<RwLock<GraphLiveliness>>,
    tel_prod_rate: Duration,
) -> Result<(), Box<dyn Error>> {
    loop {
        let is_stopped = rs.read().check_is_stopped(now, timeout);
        if let Some(shutdown) = is_stopped {
            let is_unclean = shutdown.eq(&GraphLivelinessState::StoppedUncleanly);
            rs.write().state = shutdown;
            if is_unclean {
                let voter_count = rs.read().votes.len();
                warn!(
                    "graph stopped uncleanly ({} voters); per-voter breakdown at DEBUG (e.g. RUST_LOG=steady_state=debug)",
                    voter_count
                );
                report_votes(&mut rs.write());
                return Err("graph stopped uncleanly error from watch_shutdown".into());
            }
            return Ok(());
        } else if now.elapsed() > timeout {
            warn!(
                "watch_shutdown timed out before StopRequested (state={:?})",
                rs.read().state
            );
            rs.write().state = GraphLivelinessState::StoppedUncleanly;
            return Err("graph stopped uncleanly error from watch_shutdown".into());
        } else {
            thread::sleep(tel_prod_rate);
            GraphLiveliness::vote_for_the_dead(rs.clone());
        }
    }
}

/// Logs the results of the shutdown voting process for debugging.
// ss[related graph.for-testing]
fn report_votes(state: &mut RwLockWriteGuard<GraphLiveliness>) {
    debug!("voter log: (approved votes at the top, total:{})", state.votes.len());
    let mut voters = state.votes.iter().map(|f| f.try_lock()).collect::<Vec<_>>();
    voters.sort_by_key(|voter| !voter.as_ref().is_some_and(|f| f.in_favor));
    voters.iter().for_each(|voter| {
        debug!("#{:?} Status:{:?} Voted: {:?} {:?} Ident: {:?}"
               , voter.as_ref().map_or(usize::MAX, |f| f.id)
               , voter.as_ref().map_or(Default::default(), |f| f.voter_status.clone())
               , voter.as_ref().is_some_and(|f| f.in_favor)
               , if voter.as_ref().is_some_and(|f| f.in_favor)
                            {"".to_string()} else
                            {voter.as_ref().map_or(None, |f| f.veto_reason.clone()).map_or("".to_string(), |f| f.veto_reason())}
               , voter.as_ref().map_or(Default::default(), |f| f.signature));
    });
    debug!("graph stopped uncleanly, with voters {}", voters.len());
    voters.iter().for_each(|voter| {
        let signature = voter.as_ref().map_or(&None, |f| &f.signature);
        let skip_internal = if let Some(signature) = signature {
            (metrics_server::NAME == signature.label.name) || (metrics_collector::NAME == signature.label.name)
        } else {
            false
        };
        if !skip_internal {
            let backtrace = voter.as_ref().map_or(&None, |f| &f.veto_backtrace);
            let is_veto = !voter.as_ref().is_some_and(|f| f.in_favor);
            if is_veto {
                let reason = voter.as_ref().map_or(&None, |f| &f.veto_reason);
                if let Some(r) = reason {
                    debug!("veto expression: {:#?}", r.veto_reason());
                }
                if let Some(bt) = backtrace {
                    let text = format!("{:#?}", bt);
                    let adj = text.trim();
                    let adj = adj.strip_prefix("Backtrace ").unwrap_or(adj);
                    let adj = adj.strip_prefix("[").unwrap_or(adj).trim();
                    let adj = adj.strip_suffix("]").unwrap_or(adj).trim();
                    let mut level = 1;
                    let mut is_header = true;
                    let mut start = 0;
                    for (i, c) in adj.char_indices() {
                        if c == '{' {
                            level += 1;
                        } else if c == '}' {
                            level -= 1;
                        }
                        if c == ',' && level == 1 {
                            let end = i;
                            let frame = &adj[start..end];
                            let frame = frame.trim();
                            if is_header
                                && !frame.starts_with("{ fn: \"std::backtrace")
                                && !frame.contains("GraphLiveliness::is_running")
                                && !frame.starts_with("{ fn: \"steady_state::commander_") {
                                is_header = false;
                            }
                            if !is_header {
                                debug!("{}", frame);
                                if frame.starts_with("{ fn: \"steady_state::actor_builder::launch_actor") {
                                    break;
                                }
                            }
                            start = i + 1;
                        }
                    }
                }
                debug!("\n\n");
            }
        }
    });
}

#[cfg(test)]
#[path = "shutdown_proptest.rs"]
// ss[related graph.block-until-stopped]
mod shutdown_proptest;
