//! Builtin HTTP telemetry server actor (DOT/Prometheus).
//!
//! Invariant: diagram updates apply local mCPU first, then refresh graph-share Avg load for all
//! nodes (see `docs/arch/002-telemetry-graph-share-load.md`). Keep this file under the 1,200-line budget;
//! HTTP/bind tests live in `metrics_server_tests.rs`.

// ss[impl telemetry.builtin-server]
use async_io::Async;
// ss[related philosophy.structural-hierarchy]
use std::error::Error;
// ss[related philosophy.structural-hierarchy]
use std::net::{SocketAddr, TcpListener};
// ss[impl telemetry.builtin-server]
use std::pin::Pin;
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};
// ss[related philosophy.structural-hierarchy]
use bytes::{Bytes, BytesMut};
#[allow(unused_imports)]
// ss[impl telemetry.builtin-server]
use log::*;
// ss[related philosophy.structural-hierarchy]
use crate::*;
// ss[related philosophy.structural-hierarchy]
use crate::dot::{apply_node_def, build_dot, build_metric, DotGraphFrames, FrameHistory, DotState};
// ss[impl telemetry.builtin-server]
use crate::telemetry::metrics_collector::*;
// ss[related philosophy.structural-hierarchy]
use futures::io;
// ss[related philosophy.structural-hierarchy]
use futures::channel::oneshot::Receiver;
// ss[impl telemetry.builtin-server]
use std::io::Write;
// ss[related philosophy.structural-hierarchy]
use std::fmt::Write as FmtWrite;
// ss[related philosophy.structural-hierarchy]
use futures_util::{AsyncReadExt, AsyncWriteExt, FutureExt};
// ss[impl telemetry.builtin-server]
use futures::select_biased;
// ss[related philosophy.structural-hierarchy]
use crate::steady_actor_shadow::SteadyActorShadow;
// ss[related philosophy.structural-hierarchy]
use parking_lot::RwLock;
// ss[impl telemetry.builtin-server]
use std::sync::atomic::{AtomicU64, Ordering};

// The name of the metrics server actor
// ss[impl telemetry.builtin-server]
pub const NAME: &str = "metrics_server";

/// Minimum seconds between writing `logs/graph.dot` when serving `/graph.dot` (logging); shutdown always writes once.
// ss[impl telemetry.builtin-server]
const GRAPH_DOT_DISK_WRITE_INTERVAL_SECS: u64 = 10;

#[derive(Clone)]
// ss[impl telemetry.builtin-server]
struct MetricState {
    doc: Bytes,
    metric: Bytes,
    config: Bytes,
    /// Elapsed whole seconds at last `logs/graph.dot` disk write (throttle HTTP path).
    last_disk_write: Arc<AtomicU64>,
    start_time: Instant,
}




/// Runs the metrics server, which listens for incoming telemetry data and serves it via HTTP.
///
/// # Parameters
/// - `context`: The SteadyContext instance providing execution context.
/// - `rx`: The SteadyRx instance to receive DiagramData messages.
/// - `telemetry_colors`: Optional hex colors for the telemetry top bars (primary and secondary).
/// - `bundle_floor_size`: Threshold for bundling edges in DOT output.
///
/// # Returns
/// A Result indicating success or failure.
///
/// # Errors
/// This function returns an error if the server fails to start or encounters a runtime error.
// ss[impl telemetry.builtin-server]
pub(crate) async fn run(context: SteadyActorShadow, rx: SteadyRx<DiagramData>, telemetry_colors: Option<(String, String)>, bundle_floor_size: usize) -> Result<(), Box<dyn Error>> {
    
    //NOTE: we could use this to turn off the server if desired.
    let addr = Some(format!("{}:{}"
                            , steady_config::telemetry_server_ip()
                            , steady_config::telemetry_server_port()));

    let frame_rate_ms = context.frame_rate_ms;
    let ctrl = context;
    #[cfg(feature = "telemetry_on_telemetry")]
    let ctrl = ctrl.into_spotlight([&rx], []);

    internal_behavior(ctrl, frame_rate_ms, rx, addr, telemetry_colors, bundle_floor_size).await
}

/// Wakeup source for [`internal_behavior`]. Timer is listed first in [`select_biased!`] so frame
/// boundaries are not starved when the telemetry RX stays non-empty.
#[derive(Clone, Copy, Debug)]
// ss[impl telemetry.builtin-server]
enum MetricsWake {
    /// Timelord tick completed (`wait_periodic` returned `true`).
    TimelordTick,
    /// At least one `DiagramData` is available; apply only unless shutdown forces publish.
    Data,
    /// `wait_periodic` returned `false` (shutdown interrupted the wait).
    PeriodicInterrupted,
}

// ss[impl telemetry.builtin-server]
async fn internal_behavior<C : SteadyActor>(mut ctrl: C, frame_rate_ms: u64, rx: SteadyRx<DiagramData>, addr: Option<String>, telemetry_colors: Option<(String, String)>, bundle_floor_size: usize) -> Result<(), Box<dyn Error>> {
    let effective_frame_rate_ms = frame_rate_ms.max(1);
    let frame_duration = Duration::from_millis(effective_frame_rate_ms);

    let mut initial_config = String::from("{");
    if let Some((ref c1, ref c2)) = telemetry_colors {
        let _ = write!(initial_config, "\"telemetry_colors\": [\"{}\", \"{}\"],", c1, c2);
    }
    let _ = write!(initial_config, "\"refresh_rate_ms\": {}", effective_frame_rate_ms);
    initial_config.push('}');

    // Define a new instance of the state using Arc for zero-copy handoff to readers
    let state = Arc::new(RwLock::new(MetricState {
        doc: Bytes::new(),
        metric: Bytes::new(),
        config: Bytes::from(initial_config.into_bytes()),
        last_disk_write: Arc::new(AtomicU64::new(0)),
        start_time: Instant::now(),
    }));


    let (tcp_sender_tx, tcp_receiver_tx) = oneshot::channel::<Option<Duration>>();

    //Only spin up server if addr is provided, this allows for unit testing where we cannot open that port.
    if let Some(ref addr) = addr {

        let state2 = state.clone();

        let bind_result = bind_to_port(addr);
        let opt_tcp = bind_result.listener;
        if let Some(ref listener_new) = *opt_tcp {
            let bound_addr = listener_new.local_addr().expect("Unable to get local address");
            if bound_addr.port() != bind_result.requested_port {
                warn!(
                    "Default telemetry port {} in use; using port {} for this run",
                    bind_result.requested_port,
                    bound_addr.port()
                );
            }
            #[cfg(any(feature = "telemetry_server_builtin", feature = "telemetry_server_cdn"))]
            {
                let display_addr = if bound_addr.ip().is_unspecified() { &*"127.0.0.1".to_string()
                                                                    } else { &*bound_addr.ip().to_string() };
                println!("Telemetry on http://{}:{}", display_addr, bound_addr.port());
            }
            #[cfg(feature = "prometheus_metrics")]
            {
                let display_addr = if bound_addr.ip().is_unspecified() { &*"127.0.0.1".to_string()
                                                                      } else { &*bound_addr.ip().to_string() };
                println!("Prometheus can scrape on http://{}:{}/metrics", display_addr, bound_addr.port());
            }
        } else {
            warn!("skipping telemetry due to binding issues")
        }
        //TODO: NOTE: this is probably a mistake this loop should be its own actor.
        let tcp_receiver_tx_oneshot_shutdown = Arc::new(Mutex::new(tcp_receiver_tx));
        core_exec::spawn_detached(async move {
            if let Some(ref listener_new) = *opt_tcp {
                handle_new_requests(tcp_receiver_tx_oneshot_shutdown, state2, listener_new).await;
            }
        });
    }
    let mut metrics_state = DotState::default();
    metrics_state.bundle_floor_size = bundle_floor_size;
    metrics_state.telemetry_colors = telemetry_colors;
    metrics_state.refresh_rate_ms = effective_frame_rate_ms;
    let mut history = FrameHistory::new(effective_frame_rate_ms);

    let mut frames = DotGraphFrames {
        active_metric: BytesMut::new(),
        active_graph: BytesMut::new(),
        config_line: String::with_capacity(160),
        dot_scratch: String::with_capacity(512),
        hex_line: String::with_capacity(16),
        lane_color_counts: std::collections::BTreeMap::new(),
        last_generated_graph: Instant::now(),
    };
    
    // CRITICAL: Track whether we've completed the initial full drain of the channel
    // This ensures we don't generate partial graphs during startup
    let mut initial_drain_complete = false;
    
    //generate the first empty chart (but don't flush yet)
    generate_reports(&mut metrics_state, &mut history, &mut frames, false, state.clone(), false).await;


    // CRITICAL: We must call is_running BEFORE we block on any resource (like the channel lock).
    // This ensures the actor always identifies itself and registers its shutdown vote, 
    // even if the channel is contested or the system is deadlocked.
    while ctrl.is_running(&mut || {
        // Use try_lock to avoid blocking the shutdown vote if the channel is contested.
        if let Some(mut rxg) = rx.try_lock() {
            i!(rxg.is_closed_and_empty())
        } else {
            false // Veto if we can't check, but we've identified ourselves
        }
    }) {
        // Now that we've voted to keep running, we can safely block on the lock.
        let mut rxg = rx.lock().await;

        // 1) Race timelord frame tick (publish cadence) vs data arrival (apply cadence).
        //    Timer branch is first so a due tick is not starved when the RX stays ready.
        let wake = select_biased! {
            tick_done = ctrl.wait_periodic(frame_duration).fuse() => {
                if tick_done {
                    MetricsWake::TimelordTick
                } else {
                    MetricsWake::PeriodicInterrupted
                }
            },
            _data_ready = ctrl.wait_avail(&mut rxg, 1).fuse() => MetricsWake::Data,
        };

        // 2) Apply: drain all pending telemetry (catch up; may use significant CPU if backlogged).
        while let Some(msg) = ctrl.try_take(&mut rxg) {
            process_msg(msg, &mut metrics_state, &mut history, effective_frame_rate_ms).await;
        }

        // Mark initial drain complete after the first full pass (startup had no early break).
        if !initial_drain_complete {
            initial_drain_complete = true;
        }

        let flush_all = ctrl.is_liveliness_in(&[GraphLivelinessState::StopRequested, GraphLivelinessState::Stopped]);

        // 3) Publish: DOT + Prometheus only on timelord ticks or shutdown—never solely because we applied messages.
        let should_publish =
            flush_all || matches!(wake, MetricsWake::TimelordTick);

        if should_publish {
            generate_reports(
                &mut metrics_state,
                &mut history,
                &mut frames,
                flush_all,
                state.clone(),
                true
            ).await;
        }

    }
    //force all the data we may be holding to be written to history and telemetry before we exit
    generate_reports(&mut metrics_state, &mut history, &mut frames, true, state.clone(), true).await;
    {
        let doc = state.read().doc.clone();
        let _ = write_graph_dot_to_logs(&doc).await;
    }
    let timeout = ctrl.is_liveliness_shutdown_timeout();
    let _ = tcp_sender_tx.send(timeout);
    Ok(())
}


/// A trait combining `AsyncRead` and `AsyncWrite` for types that support both.
// ss[impl telemetry.builtin-server]
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}

// ss[related philosophy.structural-hierarchy]
impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}

/// A trait for asynchronous listeners that can accept connections and provide their local address.
// ss[impl telemetry.builtin-server]
pub trait AsyncListener {
    /// Accepts a new connection asynchronously.
    ///
    /// Returns a future resolving to a stream implementing `AsyncReadWrite` and an optional socket address.
    #[allow(clippy::type_complexity)]
    // ss[impl telemetry.builtin-server]
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output =std::io::Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>)>> + Send + 'a>>;

    /// Returns the local address of the listener.
    // ss[impl telemetry.builtin-server]
    fn local_addr(&self) -> std::io::Result<SocketAddr>;
}

/// Implements `AsyncListener` for `Async<TcpListener>` to ensure true asynchronous acceptance.
// ss[impl telemetry.builtin-server]
impl AsyncListener for Async<TcpListener> {
    // ss[related philosophy.structural-hierarchy]
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output =std::io::Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>)>> + Send + 'a>> {
        Box::pin(async move {
            let (stream, addr) = self.accept().await?;
            Ok((Box::new(stream) as Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Some(addr)))
        })
    }

    // ss[impl telemetry.builtin-server]
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.get_ref().local_addr()
    }
}

// ## Utility Functions

/// Outcome of attempting to bind the telemetry HTTP listener.
// ss[impl telemetry.builtin-server]
pub(crate) struct BindToPortResult {
    pub listener: Arc<Option<Box<dyn AsyncListener + Send + Sync>>>,
    pub requested_port: u16,
}

// ss[impl telemetry.builtin-server]
fn parse_host_port(addr: &str) -> Option<(String, u16)> {
    let (host, port_str) = addr.rsplit_once(':')?;
    let port = port_str.parse::<u16>().ok()?;
    Some((host.to_string(), port))
}

// ss[impl telemetry.builtin-server]
fn try_bind_host_port(host: &str, port: u16) -> Result<Box<dyn AsyncListener + Send + Sync>, std::io::Error> {
    let addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&addr)?;
    match Async::new(listener) {
        Ok(async_listener) => Ok(Box::new(async_listener) as Box<dyn AsyncListener + Send + Sync>),
        Err(e) => {
            warn!("Unable to create async listener for http://{}: {}", addr, e);
            Err(std::io::Error::other(e))
        }
    }
}

/// Binds a TCP listener to the specified address using `Async<TcpListener>`.
///
/// When the default telemetry port is in use (env unset), increments the port by one
/// and retries until a free port is found or scan limits are reached.
///
/// Returns an `Arc` containing the listener if successful, or `None` if binding fails.
// ss[impl telemetry.builtin-server]
pub fn bind_to_port(addr: &str) -> BindToPortResult {
    let (host, start_port) = match parse_host_port(addr) {
        Some(parsed) => parsed,
        None => {
            warn!("Unable to bind to http://{}: invalid address", addr);
            return BindToPortResult {
                listener: Arc::new(None),
                requested_port: 0,
            };
        }
    };

    let scan = steady_config::telemetry_port_scan_enabled(start_port);
    let mut port = start_port;
    let mut walked: u16 = 0;
    let mut exhausted_addr_in_use = false;

    loop {
        if scan && port >= steady_config::TELEMETRY_PORT_SCAN_CEILING {
            exhausted_addr_in_use = true;
            break;
        }

        match try_bind_host_port(&host, port) {
            Ok(listener) => {
                return BindToPortResult {
                    listener: Arc::new(Some(listener)),
                    requested_port: start_port,
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if !scan {
                    warn!("Unable to bind to http://{}:{}: {}", host, port, e);
                    return BindToPortResult {
                        listener: Arc::new(None),
                        requested_port: start_port,
                    };
                }
                exhausted_addr_in_use = true;
                if walked >= steady_config::TELEMETRY_PORT_SCAN_MAX_WALK {
                    break;
                }
                port = port.saturating_add(1);
                walked = walked.saturating_add(1);
            }
            Err(e) => {
                warn!("Unable to bind to http://{}:{}: {}", host, port, e);
                return BindToPortResult {
                    listener: Arc::new(None),
                    requested_port: start_port,
                };
            }
        }
    }

    if exhausted_addr_in_use {
        warn!(
            "Unable to bind telemetry on ports {}-{} ({} ports tried): address already in use",
            start_port,
            port,
            walked.saturating_add(1)
        );
    }

    BindToPortResult {
        listener: Arc::new(None),
        requested_port: start_port,
    }
}



// ss[impl telemetry.builtin-server]
async fn handle_new_requests (
    tcp_receiver_tx_oneshot_shutdown: Arc<Mutex<Receiver<Option<Duration>>>>,
    state: Arc<RwLock<MetricState>>,
    listener: &Box<dyn AsyncListener + Send + Sync>,
) {
    //NOTE: this server is fast but only does 1 request/response at a time. This is good enough
    //      for per/second metrics and many telemetry observers with slower refresh rates
    let shutdown_wait;// = None;
    loop {
        let mut shutdown = tcp_receiver_tx_oneshot_shutdown.lock().await;
        select! {
               timeout = shutdown.deref_mut() => {

                        let margin = Duration::from_millis(10);
                        let adjusted_timeout:Duration = if let Ok(Some(timeout)) = timeout {
                                       if timeout>margin {
                                            timeout-margin
                                       } else {
                                            Duration::ZERO
                                       }
                             } else {
                                 Duration::ZERO
                            };
                        shutdown_wait = Some(adjusted_timeout);
                        break;
                    },
                result = listener.accept().fuse() => {
                    match result {
                       Ok((stream, _peer_addr)) => {
                           let state_clone = state.clone();
                           // SPAWN detached to prevent slow clients from blocking telemetry updates
                           core_exec::spawn_detached(async move {
                               let _ = handle_request(stream, state_clone).await;
                           });
                       }
                       Err(e) => {
                           //this may happen on shutdown. TODO: should not report during shutdown.
                           trace!("Error accepting connection: {}, check network configuration.",e);
                       }
                   }
           } }
    }
    if let Some(duration) = shutdown_wait {
        if duration > Duration::ZERO {
            select! {
                // Wait for the duration to expire
                _ = Delay::new(duration).fuse() => {
                    // Duration expired, exit
                }
                // Accept and handle one more connection if it arrives
                result = listener.accept().fuse() => {
                    match result {
                        Ok((stream, _peer_addr)) => {
                            let state_clone = state.clone();
                            core_exec::spawn_detached(async move {
                                let _ = handle_request(stream, state_clone).await;
                            });
                        }
                        Err(e) => {
                            error!("Error accepting connection: {}", e);
                        }
                    }
                }
            }
        }
        // If duration is zero, skip the select and exit immediately
    }
}

/// Processes a single telemetry message to update the internal `DotState`.
/// This function is designed to be called rapidly in a loop to consume bursts.
// ss[impl telemetry.builtin-server]
async fn process_msg(
    msg: DiagramData,
    metrics_state: &mut DotState,
    history: &mut FrameHistory,
    frame_rate_ms: u64,
) {
    match msg {
        DiagramData::NodeDef(seq, defs) => {
            if steady_config::TELEMETRY_HISTORY {
                let id = defs.0.ident.id;
                let name = defs.0.ident.label.name;
                history.apply_node(name, id, &defs.1, &defs.2);
            }
            apply_node_def(metrics_state, defs.0, &defs.1, &defs.2, frame_rate_ms);
            metrics_state.seq = seq;
        },
        DiagramData::NodeProcessData(_seq, actor_status) => {
            let mut touched = Vec::with_capacity(actor_status.len());
            for status in actor_status.iter() {
                assert!(
                    status.unit_total_ns >= status.await_total_ns,
                    "unit_total_ns:{:?} await_total_ns:{:?}",
                    status.unit_total_ns,
                    status.await_total_ns
                );
                let ident = status.ident;

                if let Some(node) = metrics_state.nodes.get_mut(ident.id) {
                    node.apply_local_mcpu(*status);
                    touched.push(ident.id);
                }
            }
            if !touched.is_empty() {
                metrics_state.refresh_actor_loads(&touched);
            }
        },
        DiagramData::NodeDotSubtitle(_seq, pairs) => {
            for (id, opt) in pairs.iter() {
                if let Some(node) = metrics_state.nodes.get_mut(*id) {
                    node.dot_subtitle = opt.clone();
                }
            }
        },
        DiagramData::ChannelVolumeData(seq, sparse_data) => {
            sparse_data.iter().for_each(|(id, t, s)| {
                if let Some(edge) = metrics_state.edges.get_mut(*id) {
                   edge.compute_and_refresh(*s, *t);
                }
            });
            metrics_state.seq = seq;

            if steady_config::TELEMETRY_HISTORY {
                let dense_data: Vec<(i64, i64)> = metrics_state.edges.iter()
                    .map(|e| (e.stats_computer.last_take, e.stats_computer.last_send))
                    .collect();
                history.apply_edge(&dense_data, frame_rate_ms);
            }
        },
    }
}

// ss[impl telemetry.builtin-server]
async fn generate_reports(metrics_state: &mut DotState, history: &mut FrameHistory, frames: &mut DotGraphFrames, flush_all: bool, state: Arc<RwLock<MetricState>>, flush_frame: bool) {
    if steady_config::TELEMETRY_HISTORY {
        history.update(flush_all).await;
        history.mark_position();
    }

    if flush_frame {
        build_dot(metrics_state, frames);
        let cap_g = frames.active_graph.capacity();
        let built_graph = std::mem::replace(&mut frames.active_graph, BytesMut::with_capacity(cap_g));
        let graph_bytes = built_graph.freeze();

        build_metric(metrics_state, &mut frames.active_metric);
        let cap_m = frames.active_metric.capacity();
        let built_metric = std::mem::replace(&mut frames.active_metric, BytesMut::with_capacity(cap_m));
        let metric_bytes = built_metric.freeze();

        let cap_cfg = frames.config_line.capacity();
        frames.config_line.clear();
        let _ = write!(frames.config_line, "{{");
        if let Some((ref c1, ref c2)) = metrics_state.telemetry_colors {
            let _ = write!(frames.config_line, "\"telemetry_colors\": [\"{}\", \"{}\"],", c1, c2);
        } else {
            let _ = write!(frames.config_line, "\"telemetry_colors\": [\"#00a900\", \"#008000\"],");
        }
        let _ = write!(frames.config_line, "\"refresh_rate_ms\": {}", metrics_state.refresh_rate_ms);
        frames.config_line.push('}');
        let config_bytes = Bytes::from(
            std::mem::replace(&mut frames.config_line, String::with_capacity(cap_cfg)).into_bytes(),
        );

        // SWAP: Hold the write lock only for the duration of pointer updates
        // This ensures that readers (HTTP handlers) are never blocked by the heavy DOT generation.
        {
            let mut state_guard = state.write();
            state_guard.doc = graph_bytes;
            state_guard.metric = metric_bytes;
            state_guard.config = config_bytes;
        }
        frames.last_generated_graph = Instant::now();
    }
}

// --- Telemetry assets embedded at compile time (gzip or raw bytes) ---------------------------
//
// **Never** embed build-generated telemetry blobs using paths under `../../target/...` or only
// `CARGO_TARGET_DIR`. Those paths are not guaranteed to match where `build.rs` wrote the files
// (workspace `target/` vs `core/target/`), which once produced binaries that served empty
// `viz-lite.js` and broke `importScripts` in the web worker.
//
// `build.rs` writes the following **flat basenames** into Cargo’s `OUT_DIR`; `env!("OUT_DIR")`
// here resolves to that same directory when `rustc` compiles this crate. Keep names in sync with
// `build.rs` (`viz-lite.js.gz`, `index.html.gz`, `webworker.js.gz`, `dot-viewer.js.gz`,
// `dot-viewer.css.gz`, `spinner.gif`).
//
// Repo-tracked SVGs under `../../static/telemetry/...` stay on those paths — they are not
// `build.rs` outputs and are not placed in `OUT_DIR`.
#[allow(dead_code)]
#[cfg(any(docsrs, feature = "telemetry_server_cdn", not(feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_VIZ_LITE_GZ: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(any(docsrs, feature = "telemetry_server_cdn")), feature = "telemetry_server_builtin"))]
// ss[impl telemetry.builtin-server]
const CONTENT_VIZ_LITE_GZ: & [u8] = if steady_config::TELEMETRY_SERVER {
    include_bytes!(concat!(env!("OUT_DIR"), "/viz-lite.js.gz"))
} else {
    &[]
};

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_INDEX_HTML_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_INDEX_HTML_GZ: & [u8] = if steady_config::TELEMETRY_SERVER {
    include_bytes!(concat!(env!("OUT_DIR"), "/index.html.gz"))
} else {
    &[]
};

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_DOT_VIEWER_JS_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
// ss[impl telemetry.live-title]
const CONTENT_DOT_VIEWER_JS_GZ: & [u8] = if steady_config::TELEMETRY_SERVER {
    include_bytes!(concat!(env!("OUT_DIR"), "/dot-viewer.js.gz"))
} else {
    &[]
};

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_DOT_VIEWER_CSS_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_DOT_VIEWER_CSS_GZ: & [u8] = if steady_config::TELEMETRY_SERVER {
    include_bytes!(concat!(env!("OUT_DIR"), "/dot-viewer.css.gz"))
} else {
    &[]
};

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_WEBWORKER_JS_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
// ss[impl telemetry.live-title]
const CONTENT_WEBWORKER_JS_GZ: & [u8] = if steady_config::TELEMETRY_SERVER {
    include_bytes!(concat!(env!("OUT_DIR"), "/webworker.js.gz"))
} else {
    &[]
};

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_SPINNER_GIF_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_SPINNER_GIF: & [u8] = if steady_config::TELEMETRY_SERVER {
    include_bytes!(concat!(env!("OUT_DIR"), "/spinner.gif"))
} else {
    &[]
};

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_PREVIEW_ICON_SVG: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_PREVIEW_ICON_GZ: & [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../static/telemetry/images/preview-icon.svg") } else { &[] };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/refresh-time-icon.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_USER_ICON_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_USER_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/user-icon.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_ZOOM_IN_ICON_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_ZOOM_IN_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon-disabled.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon-disabled.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
// ss[impl telemetry.builtin-server]
const CONTENT_DOWNLOAD_ICON_PNG: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
// ss[impl telemetry.builtin-server]
const CONTENT_DOWNLOAD_ICON_PNG: & [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../static/telemetry/images/download.png") } else { &[] };

//   pub trait AsyncWriteExt: AsyncWrite   for the .read
//   pub trait AsyncReadExt: AsyncRead     for the .write_all

/// Writes the current graph DOT bytes to `logs/graph.dot` (blocking I/O on the pool).
// ss[impl telemetry.builtin-server]
async fn write_graph_dot_to_logs(data: &[u8]) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all("logs");
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("logs/graph.dot")
    {
        let data_buf = BytesMut::from(data);
        async_write_all(data_buf, true, file).await?;
    }
    Ok(())
}

// ss[impl telemetry.builtin-server]
async fn handle_request<T>(mut stream: T,
                           state: Arc<RwLock<MetricState>>) -> io::Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin
{

    let mut buffer = vec![0; 1024];
    let _ = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer);

    // Parse the HTTP request to get the method and path
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");

    // Define the CORS header to include in all responses
    let cors_header = "Access-Control-Allow-Origin: *\r\n";

    // Handle OPTIONS requests for CORS preflight
    if method == "OPTIONS" {
        stream.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\n\r\n").await?;
        return Ok(());
    }

    #[cfg(feature = "prometheus_metrics")]
    if path.starts_with("/me") { // for prometheus /metrics
        let metric = state.read().metric.clone(); // Arc clone is instant
        stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: text/plain\r\nContent-Length: ", cors_header).as_bytes()).await?;
        stream.write_all(itoa::Buffer::new().format(metric.len()).as_bytes()).await?;
        stream.write_all(b"\r\n\r\n").await?;
        stream.write_all(&metric).await?;
        return Ok(());
    }

    #[cfg(any(feature = "telemetry_server_builtin", feature = "telemetry_server_cdn"))]
    {
        if path.starts_with("/gr") { // for local telemetry /graph.dot

            let (data, can_write) = {
                let locked_state = state.read(); // Use READ lock
                let now_secs = locked_state.start_time.elapsed().as_secs();
                let last_secs = locked_state.last_disk_write.load(Ordering::Relaxed);
                
                let can_write =
                    now_secs >= last_secs.saturating_add(GRAPH_DOT_DISK_WRITE_INTERVAL_SECS);
                if can_write {
                    locked_state.last_disk_write.store(now_secs, Ordering::Relaxed);
                }
                (locked_state.doc.clone(), can_write)
            };

            stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: text/vnd.graphviz\r\nContent-Length: ", cors_header).as_bytes()).await?;
            stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
            stream.write_all(b"\r\n\r\n").await?;
            stream.write_all(&data).await?;
            stream.flush().await?;

            if can_write {
                let _ = write_graph_dot_to_logs(&data).await;
            }

            return Ok(());
        } else if path.starts_with("/co") { // for /config
            let config = state.read().config.clone(); // Arc clone is instant
            stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: application/json\r\nContent-Length: ", cors_header).as_bytes()).await?;
            stream.write_all(itoa::Buffer::new().format(config.len()).as_bytes()).await?;
            stream.write_all(b"\r\n\r\n").await?;
            stream.write_all(&config).await?;
            return Ok(());
        } else if path.starts_with("/set?") { // example /set?rankdir=LR&show=label1,label2&hide=label3,label4
            let mut parts = path.split("?");
            if let Some(_part) = parts.next() {
                if let Some(part) = parts.next() {
                    let parts = part.split("&");
                    for part in parts {
                        let mut _parts = part.split("=");
                        // if let Some(key) = parts.next() {
                        //     if let Some(value) = parts.next() {
                        //         if "rankdir" == key {
                        //             rankdir = value;
                        //         }
                        //     }
                        // }
                    }
                }
            }
            // if rankdir.eq("LR") || rankdir.eq("TB") {
            //     _config.rankdir = rankdir.to_string();
            //     // TODO: Labels feature (commented out in original code)
            //     // if c.apply_labels(show, hide) {
            //     //     stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Length: 0\r\n\r\n", cors_header).as_bytes()).await?;
            //     //     return Ok(());
            //     // }
            // }
            stream.write_all(format!("HTTP/1.1 400 Bad Request\r\n{}Content-Length: 0\r\n\r\n", cors_header).as_bytes()).await?;
            return Ok(());
        } else if path.eq("/") || path.starts_with("/in") || path.starts_with("/de") { // index
            stream.write_all(format!("HTTP/1.1 200 OK\r\n{}content-encoding: gzip\r\nContent-Type: text/html\r\nContent-Length: ", cors_header).as_bytes()).await?;
            stream.write_all(itoa::Buffer::new().format(CONTENT_INDEX_HTML_GZ.len()).as_bytes()).await?;
            stream.write_all(b"\r\n\r\n").await?;
            stream.write_all(CONTENT_INDEX_HTML_GZ).await?;
            return Ok(());
        } else if path.starts_with("/im") && path.len().ge(&15) { // /images/*
            if path.as_bytes()[8].eq(&b'z') {
                if path.as_bytes()[13].eq(&b'i') {
                    if path.len().ge(&30) { // "/images/zoom-in-icon-disabled.svg"
                        let data = CONTENT_ZOOM_IN_ICON_DISABLED_SVG.as_bytes();
                        stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: image/svg+xml\r\nContent-Length: ", cors_header).as_bytes()).await?;
                        stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        stream.write_all(data).await?;
                    } else { // "/images/zoom-in-icon.svg"
                        let data = CONTENT_ZOOM_IN_ICON_SVG.as_bytes();
                        stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: image/svg+xml\r\nContent-Length: ", cors_header).as_bytes()).await?;
                        stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        stream.write_all(data).await?;
                    }
                } else if path.len().ge(&30) { // "/images/zoom-out-icon-disabled.svg"
                    let data = CONTENT_ZOOM_OUT_ICON_DISABLED_SVG.as_bytes();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: image/svg+xml\r\nContent-Length: ", cors_header).as_bytes()).await?;
                    stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    stream.write_all(data).await?;
                } else { // "/images/zoom-out-icon.svg"
                    let data = CONTENT_ZOOM_OUT_ICON_SVG.as_bytes();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: image/svg+xml\r\nContent-Length: ", cors_header).as_bytes()).await?;
                    stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    stream.write_all(data).await?;
                }
            } else if path.as_bytes()[11].eq(&b'r') {
                if path.len().ge(&22) { // "/images/refresh-time-icon.svg"
                    let data = CONTENT_REFRESH_TIME_ICON_SVG.as_bytes();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: image/svg+xml\r\nContent-Length: ", cors_header).as_bytes()).await?;
                    stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    stream.write_all(data).await?;
                } else { // "/images/user-icon.svg"
                    let data = CONTENT_USER_ICON_SVG.as_bytes();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: image/svg+xml\r\nContent-Length: ", cors_header).as_bytes()).await?;
                    stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    stream.write_all(data).await?;
                }
            } else if path.as_bytes()[8].eq(&b'd') { // "/images/download.png"
                stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: image/png\r\nContent-Length: ", cors_header).as_bytes()).await?;
                stream.write_all(itoa::Buffer::new().format(CONTENT_DOWNLOAD_ICON_PNG.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                stream.write_all(CONTENT_DOWNLOAD_ICON_PNG).await?;
            } else if path.len().ge(&22) { // "/images/preview-icon.svg"
                stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: image/svg+xml\r\nContent-Length: ", cors_header).as_bytes()).await?;
                stream.write_all(itoa::Buffer::new().format(CONTENT_PREVIEW_ICON_GZ.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                stream.write_all(CONTENT_PREVIEW_ICON_GZ).await?;
            } else { // "/images/spinner.gif"
                stream.write_all(format!("HTTP/1.1 200 OK\r\n{}Content-Type: image/gif\r\nContent-Length: ", cors_header).as_bytes()).await?;
                stream.write_all(itoa::Buffer::new().format(CONTENT_SPINNER_GIF.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                stream.write_all(CONTENT_SPINNER_GIF).await?;
            }
            return Ok(());
        } else if path.starts_with("/we") { // "/webworker.js"
            stream.write_all(format!("HTTP/1.1 200 OK\r\n{}content-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ", cors_header).as_bytes()).await?;
            stream.write_all(itoa::Buffer::new().format(CONTENT_WEBWORKER_JS_GZ.len()).as_bytes()).await?;
            stream.write_all(b"\r\n\r\n").await?;
            stream.write_all(CONTENT_WEBWORKER_JS_GZ).await?;
            return Ok(());
        } else if path.starts_with("/do") {
            if path.ends_with(".css") { // "/dot-viewer.css"
                stream.write_all(format!("HTTP/1.1 200 OK\r\n{}content-encoding: gzip\r\nContent-Type: text/css\r\nContent-Length: ", cors_header).as_bytes()).await?;
                stream.write_all(itoa::Buffer::new().format(CONTENT_DOT_VIEWER_CSS_GZ.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                stream.write_all(CONTENT_DOT_VIEWER_CSS_GZ).await?;
            } else { // "/dot-viewer.js"
                stream.write_all(format!("HTTP/1.1 200 OK\r\n{}content-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ", cors_header).as_bytes()).await?;
                stream.write_all(itoa::Buffer::new().format(CONTENT_DOT_VIEWER_JS_GZ.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                stream.write_all(CONTENT_DOT_VIEWER_JS_GZ).await?;
            }
            return Ok(());
        } else if path.starts_with("/vi") { // "/viz-lite.js"
            stream.write_all(format!("HTTP/1.1 200 OK\r\n{}content-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ", cors_header).as_bytes()).await?;
            stream.write_all(itoa::Buffer::new().format(CONTENT_VIZ_LITE_GZ.len()).as_bytes()).await?;
            stream.write_all(b"\r\n\r\n").await?;
            stream.write_all(CONTENT_VIZ_LITE_GZ).await?;
            return Ok(());
        } else {
            stream.write_all(format!("HTTP/1.1 404 Not Found\r\n{}Content-Length: 0\r\n\r\n", cors_header).as_bytes()).await?;
            return Ok(());
        }
    }

    #[allow(unreachable_code)]
    {
        stream.write_all(format!("HTTP/1.1 404 Not Found\r\n{}Content-Length: 0\r\n\r\n", cors_header).as_bytes()).await?;
        Ok(())
    }
}

#[cfg(not(windows))]

/// Asynchronously writes data to a file, optionally flushing it.
///
/// Uses `spawn_blocking` to perform blocking file I/O in a separate thread.
// ss[impl telemetry.builtin-server]
pub(crate) async fn async_write_all(data: BytesMut, flush: bool, mut file: std::fs::File) -> std::io::Result<()> {
    core_exec::spawn_blocking(move || {
        file.write_all(&data)?;
        if flush {
            file.flush()?;
        }
        Ok(())
    }).await
}

#[cfg(test)]
// ss[impl telemetry.builtin-server]
#[path = "metrics_server_tests.rs"]
mod metrics_server_tests;
