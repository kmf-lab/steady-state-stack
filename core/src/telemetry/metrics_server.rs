use std::error::Error;
use std::process::{exit};
use bytes::BytesMut;
#[allow(unused_imports)]
use log::*;
use crate::*;
use crate::dot::{build_dot, DotGraphFrames, MetricState, FrameHistory, apply_node_def, build_metric};
use crate::telemetry::metrics_collector::*;
use futures::io;
use nuclei::*;
use std::net::{TcpListener, TcpStream};

// The name of the metrics server actor
pub const NAME: &str = "metrics_server";

#[derive(Clone)]
struct State {
    doc: Vec<u8>,
    metric: Vec<u8>,
}

/// Runs the metrics server, which listens for incoming telemetry data and serves it via HTTP.
///
/// # Parameters
/// - `context`: The SteadyContext instance providing execution context.
/// - `rx`: The SteadyRx instance to receive DiagramData messages.
///
/// # Returns
/// A Result indicating success or failure.
///
/// # Errors
/// This function returns an error if the server fails to start or encounters a runtime error.
pub(crate) async fn run(context: SteadyContext, rx: SteadyRx<DiagramData>) -> std::result::Result<(), Box<dyn Error>> {
    let ctrl = context;
    #[cfg(feature = "telemetry_on_telemetry")]
    let mut ctrl = into_monitor!(ctrl, [rx], []);

    let mut rxg = rx.lock().await;
    let mut metrics_state = MetricState::default();
    let top_down = false;
    let rankdir = if top_down { "TB" } else { "LR" };
    let mut frames = DotGraphFrames {
        active_metric: BytesMut::new(),
        last_graph: Instant::now(),
        active_graph: BytesMut::new(),
    };

    // Define a new instance of the state.
    let state = Arc::new(Mutex::new(State {
        doc: Vec::new(),
        metric: Vec::new(),
    }));

    let mut history = FrameHistory::new(ctrl.frame_rate_ms);

    let addr = format!("{}:{}", steady_config::telemetry_server_ip(), steady_config::telemetry_server_port());

    let (tcp_sender_tx, tcp_receiver_tx) = oneshot::channel();
    let tcp_receiver_tx_oneshot_shutdown = Arc::new(Mutex::new(tcp_receiver_tx));

    //TODO: this hack spawn block will go to spawn_local after nuclei is fixed.
    // if I use spawn_local this blocks all other work on my thread but I am not sure why.
    let state2 = state.clone();
    nuclei::spawn(async move {
        let listener_new = Handle::<TcpListener>::bind(addr).expect("Unable to Bind"); //TODO: need better error.
        #[cfg(any(feature = "telemetry_server_builtin", feature = "telemetry_server_cdn"))]
        println!("Telemetry on http://{}", listener_new.get_ref().local_addr().expect("Unable to read local address"));

        #[cfg(feature = "prometheus_metrics")]
        println!("Prometheus can scrape on on http://{}/metrics", listener_new.get_ref().local_addr().expect("Unable to read local address"));

        //NOTE: this server is fast but only does 1 request/response at a time. This is good enough
        //      for per/second metrics and many telemetry observers with slower refresh rates
        loop {
            let mut shutdown = tcp_receiver_tx_oneshot_shutdown.lock().await;
            select! {  _ = shutdown.deref_mut() => {  break;  },
                          result = listener_new.accept().fuse() => {
                         match result {
                            Ok((stream, _peer_addr)) => {
                                let _ = handle_request(stream,state2.clone()).await;
                            }
                            Err(e) => {
                                error!("Error accepting connection: {}",e);
                            }
                        }
                    } };
        }
    }).detach();

    while ctrl.is_running(&mut || rxg.is_empty() && rxg.is_closed()) {

        let _clean = wait_for_all!(
                                    ctrl.wait_avail_units(&mut rxg,1)
                                  ).await;

        if let Some(msg) = ctrl.try_take(&mut rxg) {
            let rate = ctrl.frame_rate_ms;
            let flush = ctrl.is_liveliness_in(&[GraphLivelinessState::StopRequested, GraphLivelinessState::Stopped], true);
            process_msg(msg
                        , &mut metrics_state
                        , &mut history
                        , &mut frames
                        , rate
                        , flush
                        , &rxg
                        , rankdir
                        , state.clone()).await;
            #[cfg(feature = "telemetry_on_telemetry")]
            ctrl.relay_stats_smartly();
        }

        #[cfg(feature = "telemetry_on_telemetry")]
        ctrl.relay_stats_smartly();

    }

    tcp_sender_tx.send(());


    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn process_msg(msg: DiagramData
                     , metrics_state: &mut MetricState
                     , history: &mut FrameHistory
                     , frames: &mut DotGraphFrames
                     , frame_rate_ms: u64
                     , flush_all: bool
                     , rxg: &MutexGuard<'_, Rx<DiagramData>>
                     , rankdir: &str
                     , state: Arc<Mutex<State>>

) {
    match msg {
        DiagramData::NodeDef(seq, defs) => {
            let name = defs.0.name;
            let id = defs.0.id;
            apply_node_def(metrics_state, name, id, defs.0, &defs.1, &defs.2, frame_rate_ms);
            metrics_state.seq = seq;
            if steady_config::TELEMETRY_HISTORY {
                history.apply_node(name, id, &defs.1, &defs.2);
            }
        },
        DiagramData::NodeProcessData(_seq, actor_status) => {
            let total_work_ns: u128 = actor_status.iter().map(|status| {
                assert!(status.unit_total_ns >= status.await_total_ns, "unit_total_ns:{} await_total_ns:{}", status.unit_total_ns, status.await_total_ns);
                (status.unit_total_ns - status.await_total_ns) as u128
            }).sum();

            actor_status.iter().enumerate().for_each(|(i, status)| {
                #[cfg(debug_assertions)]
                if metrics_state.nodes.is_empty() { exit(-1); }
                assert!(!metrics_state.nodes.is_empty());
                metrics_state.nodes[i].compute_and_refresh(*status, total_work_ns);
            });
        },
        DiagramData::ChannelVolumeData(seq, total_take_send) => {
            total_take_send.iter().enumerate().for_each(|(i, (t, s))| {
                #[cfg(debug_assertions)]
                if metrics_state.edges.is_empty() { exit(-1); }
                assert!(!metrics_state.edges.is_empty());
                metrics_state.edges[i].compute_and_refresh(*s, *t);
            });
            metrics_state.seq = seq;

            if steady_config::TELEMETRY_HISTORY {
                history.apply_edge(&total_take_send, frame_rate_ms);

                history.update(flush_all).await;
                history.mark_position();
            }

            if rxg.is_empty() || frames.last_graph.elapsed().as_millis() > 2 * frame_rate_ms as u128 {
                build_dot(metrics_state, rankdir, &mut frames.active_graph);
                let graph_bytes = frames.active_graph.to_vec();
                build_metric(metrics_state, &mut frames.active_metric);
                let metric_bytes = frames.active_metric.to_vec();

                let mut state = state.lock().await;
                state.doc = graph_bytes;
                state.metric = metric_bytes;
                drop(state);
                frames.last_graph = Instant::now();
            }
        },
    }
}


#[cfg(any(docsrs, feature = "telemetry_server_cdn", not(feature = "telemetry_server_builtin")))]
const CONTENT_VIZ_LITE_B64: &str = "";
#[cfg(all(not(any(docsrs, feature = "telemetry_server_cdn")), feature = "telemetry_server_builtin"))]
const CONTENT_VIZ_LITE_B64: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/viz-lite.js.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_INDEX_HTML_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_INDEX_HTML_B64: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/index.html.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_DOT_VIEWER_JS_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_DOT_VIEWER_JS_B64: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/dot-viewer.js.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_DOT_VIEWER_CSS_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_DOT_VIEWER_CSS_B64: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/dot-viewer.css.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_WEBWORKER_JS_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_WEBWORKER_JS_B64: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/webworker.js.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_SPINNER_GIF_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_SPINNER_GIF_B64: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/images/spinner.gif.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_PREVIEW_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_PREVIEW_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/preview-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/refresh-time-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_USER_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_USER_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/user-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_IN_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_IN_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon-disabled.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon-disabled.svg") } else { "" };


/// Decodes a base64-encoded string into a byte vector.
///
/// # Parameters
/// - `input`: The base64-encoded string to decode.
///
/// # Returns
/// A Result containing a Vec<u8> with the decoded bytes or a String with an error message.
fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits_collected: u8 = 0;

    for c in input.chars() {
        let value = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            '=' => break,
            x => return Err(format!("Invalid character {:?} in Base64 input.", x)),
        };

        buffer = (buffer << 6) | value;
        bits_collected += 6;
        if bits_collected == 24 {
            output.push((buffer >> 16) as u8);
            output.push((buffer >> 8) as u8);
            output.push(buffer as u8);
            buffer = 0;
            bits_collected = 0;
        }
    }

    if bits_collected == 8 {
        output.push((buffer >> 4) as u8);
    } else if bits_collected == 16 {
        output.push((buffer >> 10) as u8);
        output.push((buffer >> 2) as u8);
    }

    Ok(output)
}



async fn handle_request(mut stream: Handle<TcpStream>, state: Arc<Mutex<State>>) -> io::Result<()> {
    let mut buffer = vec![0; 1024];
    let _ = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer);

    // Parse the HTTP request to get the path.
    let path = request.split_whitespace().nth(1).unwrap_or("/");

    #[cfg(feature = "prometheus_metrics")]
    if path.starts_with("/me") { //for prometheus /metrics
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: ").await?;
        let locked_state = state.lock().await;
        stream.write_all(itoa::Buffer::new().format(locked_state.metric.len()).as_bytes()).await?;
        stream.write_all(b"\r\n\r\n").await?;
        stream.write_all(&locked_state.metric).await?;
        return Ok(());
    }

    #[cfg(any(feature = "telemetry_server_builtin", feature = "telemetry_server_cdn"))]
    {
        if path.starts_with("/gr") { //for local telemetry /graph.dot
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/vnd.graphviz\r\nContent-Length: ").await?;
            let locked_state = state.lock().await;
            stream.write_all(itoa::Buffer::new().format(locked_state.doc.len()).as_bytes()).await?;
            stream.write_all(b"\r\n\r\n").await?;
            stream.write_all(&locked_state.doc).await?;
            return Ok(());
        } else if path.eq("/") || path.starts_with("/in") || path.starts_with("/de") { //index
            if let Ok(data) = decode_base64(CONTENT_INDEX_HTML_B64) {
                // Write the HTTP header
                stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/html\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                // Write the actual data
                stream.write_all(&data).await?;
            } else {
                let content = b"Failed to decode CONTENT_INDEX_HTML_B64";
                stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(content.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                stream.write_all(content).await?;
            }
            return Ok(());
        } else if path.starts_with("/im") && path.len().ge(&15) { // /images/*
            if path.as_bytes()[8].eq(&b'z') {
                if path.as_bytes()[13].eq(&b'i') {
                    if path.len().ge(&30) { //"/images/zoom-in-icon-disabled.svg"
                        let data = CONTENT_ZOOM_IN_ICON_DISABLED_SVG.as_bytes();
                        // Write the HTTP header
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: ").await?;
                        stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        // Write the actual data
                        stream.write_all(data).await?;
                    } else {                      //"/images/zoom-in-icon.svg"
                        let data = CONTENT_ZOOM_IN_ICON_SVG.as_bytes();
                        // Write the HTTP header
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: ").await?;
                        stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        // Write the actual data
                        stream.write_all(data).await?;
                    }
                } else if path.len().ge(&30) { // "/images/zoom-out-icon-disabled.svg"
                        let data = CONTENT_ZOOM_OUT_ICON_DISABLED_SVG.as_bytes();
                        // Write the HTTP header
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: ").await?;
                        stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        // Write the actual data
                        stream.write_all(data).await?;
                    } else {                     //"/images/zoom-out-icon.svg"
                        let data = CONTENT_ZOOM_OUT_ICON_SVG.as_bytes();
                        // Write the HTTP header
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: ").await?;
                        stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        // Write the actual data
                        stream.write_all(data).await?;
                    }
            } else if path.as_bytes()[11].eq(&b'r') {
                    if path.len().ge(&22) {   //"/images/refresh-time-icon.svg"
                        let data = CONTENT_REFRESH_TIME_ICON_SVG.as_bytes();
                        // Write the HTTP header
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: ").await?;
                        stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        // Write the actual data
                        stream.write_all(data).await?;
                    } else {                        //"/images/user-icon.svg"
                        let data = CONTENT_USER_ICON_SVG.as_bytes();
                        // Write the HTTP header
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: ").await?;
                        stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        // Write the actual data
                        stream.write_all(data).await?;
                    }
                } else if path.len().ge(&22) {   //"/images/preview-icon.svg"
                        let data = CONTENT_PREVIEW_ICON_SVG.as_bytes();
                        // Write the HTTP header
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: ").await?;
                        stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        // Write the actual data
                        stream.write_all(data).await?;
                    } else {                        //"/images/spinner.gif"
                        if let Ok(data) = decode_base64(CONTENT_SPINNER_GIF_B64) {
                            // Write the HTTP header
                            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: ").await?;
                            stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                            stream.write_all(b"\r\n\r\n").await?;
                            // Write the actual data
                            stream.write_all(&data).await?;
                        } else {
                            let content = b"Failed to decode CONTENT_SPINNER_GIF_B64";
                            stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html\r\nContent-Length: ").await?;
                            stream.write_all(itoa::Buffer::new().format(content.len()).as_bytes()).await?;
                            stream.write_all(b"\r\n\r\n").await?;
                            stream.write_all(content).await?;
                        }
                }

            return Ok(());
        } else if path.starts_with("/we") {         //"/webworker.js"
            if let Ok(data) = decode_base64(CONTENT_WEBWORKER_JS_B64) {
                // Write the HTTP header
                stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                // Write the actual data
                stream.write_all(&data).await?;
            } else {
                let content = b"Failed to decode CONTENT_WEBWORKER_JS_B64";
                stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(content.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                stream.write_all(content).await?;
            }
            return Ok(());
        } else if path.starts_with("/do") {
            if path.ends_with(".css") { //"/dot-viewer.css"
                if let Ok(data) = decode_base64(CONTENT_DOT_VIEWER_CSS_B64) {
                    // Write the HTTP header
                    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/css\r\nContent-Length: ").await?;
                    stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    // Write the actual data
                    stream.write_all(&data).await?;
                } else {
                    let content = b"Failed to decode CONTENT_DOT_VIEWER_CSS_B64";
                    stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html\r\nContent-Length: ").await?;
                    stream.write_all(itoa::Buffer::new().format(content.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    stream.write_all(content).await?;
                }
            } else { //"/dot-viewer.js"
                if let Ok(data) = decode_base64(CONTENT_DOT_VIEWER_JS_B64) {
                    // Write the HTTP header
                    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ").await?;
                    stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    // Write the actual data
                    stream.write_all(&data).await?;
                } else {
                    let content = b"Failed to decode CONTENT_DOT_VIEWER_JS_B64";
                    stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html\r\nContent-Length: ").await?;
                    stream.write_all(itoa::Buffer::new().format(content.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    stream.write_all(content).await?;
                }
            }
            return Ok(());
        } else if path.starts_with("/vi") {        //"/viz-lite.js"
            if let Ok(data) = decode_base64(CONTENT_VIZ_LITE_B64) {
                // Write the HTTP header
                stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(data.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                // Write the actual data
                stream.write_all(&data).await?;
            } else {
                let content = b"Failed to decode CONTENT_VIZ_LITE_B64";
                stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(content.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                stream.write_all(content).await?;
            }
            return Ok(());
        } else {
            stream.write_all("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".as_bytes()).await?;
            return Ok(());
        }
    }
    #[allow(unreachable_code)]
    {
        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await?;
        Ok(())
    }
}
