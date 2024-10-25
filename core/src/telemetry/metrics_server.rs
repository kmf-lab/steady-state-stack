use std::error::Error;
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
pub(crate) async fn run(context: SteadyContext, rx: SteadyRx<DiagramData>) -> Result<(), Box<dyn Error>> {
    let addr = Some(format!("{}:{}", steady_config::telemetry_server_ip(), steady_config::telemetry_server_port()));
    internal_behavior(context, rx, addr).await
}

async fn internal_behavior(context: SteadyContext, rx: SteadyRx<DiagramData>, addr: Option<String>) -> Result<(), Box<dyn Error>> {
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

    let (tcp_sender_tx, tcp_receiver_tx) = oneshot::channel();
    let tcp_receiver_tx_oneshot_shutdown = Arc::new(Mutex::new(tcp_receiver_tx));

    //Only spin up server if addr is provided, this allows for unit testing where we cannot open that port.
    if let Some(addr) = addr {
        let state2 = state.clone();
        //TODO: this hack spawn block will go to spawn_local after nuclei is fixed.
        // if I use spawn_local this blocks all other work on my thread but I am not sure why.
        spawn(async move {
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
                    } }
            }
        }).detach();
    }

    while ctrl.is_running(&mut || rxg.is_empty() && rxg.is_closed()) {
        let _clean = wait_for_all!( ctrl.wait_shutdown_or_avail_units(&mut rxg,1) );

        if let Some(msg) = ctrl.try_take(&mut rxg) {
            process_msg(msg
                        , &mut metrics_state
                        , &mut history
                        , &mut frames
                        , ctrl.frame_rate_ms
                        , ctrl.is_liveliness_in(&[GraphLivelinessState::StopRequested, GraphLivelinessState::Stopped])
                        , &rxg
                        , rankdir
                        , state.clone()).await;
        }
    }
    let _ = tcp_sender_tx.send(());
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
            if steady_config::TELEMETRY_HISTORY {
                let id = defs.0.ident.id;
                let name = defs.0.ident.label.name;
                history.apply_node(name, id, &defs.1, &defs.2);
            }
            apply_node_def(metrics_state, defs.0, &defs.1, &defs.2, frame_rate_ms);
            metrics_state.seq = seq;
        },
        DiagramData::NodeProcessData(_seq, actor_status) => {
            let total_work_ns: u128 = actor_status.iter().map(|status| {
                assert!(status.unit_total_ns >= status.await_total_ns, "unit_total_ns:{} await_total_ns:{}", status.unit_total_ns, status.await_total_ns);
                (status.unit_total_ns - status.await_total_ns) as u128
            }).sum();

            actor_status.iter().enumerate().for_each(|(i, status)| {
                #[cfg(debug_assertions)]
                assert!(!metrics_state.nodes.is_empty());
                metrics_state.nodes[i].compute_and_refresh(*status, total_work_ns);
            });
        },
        DiagramData::ChannelVolumeData(seq, total_take_send) => {
            total_take_send.iter().enumerate().for_each(|(i, (t, s))| {
                #[cfg(debug_assertions)]
                assert!(!metrics_state.edges.is_empty());
                metrics_state.edges[i].compute_and_refresh(*s, *t);
            });
            metrics_state.seq = seq;

            if steady_config::TELEMETRY_HISTORY {
                history.apply_edge(&total_take_send, frame_rate_ms);

                history.update(flush_all).await;
                history.mark_position();
            }

            if rxg.is_empty() || frames.last_graph.elapsed().as_millis() >= 2 * frame_rate_ms as u128 {
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
const CONTENT_VIZ_LITE_GZ: &'static [u8] = &[];
#[cfg(all(not(any(docsrs, feature = "telemetry_server_cdn")), feature = "telemetry_server_builtin"))]
const CONTENT_VIZ_LITE_GZ: &'static [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/viz-lite.js.gz") } else { &[] };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_INDEX_HTML_B64: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_INDEX_HTML_GZ: &'static [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/index.html.gz") } else { &[] };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_DOT_VIEWER_JS_B64: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_DOT_VIEWER_JS_GZ: &'static [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/dot-viewer.js.gz") } else { &[] };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_DOT_VIEWER_CSS_B64: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_DOT_VIEWER_CSS_GZ: &'static [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/dot-viewer.css.gz") } else { &[] };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_WEBWORKER_JS_B64: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_WEBWORKER_JS_GZ: &'static [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/webworker.js.gz") } else { &[] };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_SPINNER_GIF_B64: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_SPINNER_GIF: &'static [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/images/spinner.gif") } else { &[] };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_PREVIEW_ICON_SVG: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_PREVIEW_ICON_GZ: &'static [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../static/telemetry/images/preview-icon.svg") } else { &[] };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_REFRESH_TIME_ICON_SVG: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/refresh-time-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_USER_ICON_SVG: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_USER_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/user-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_IN_ICON_SVG: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_IN_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon-disabled.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_OUT_ICON_SVG: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &'static [u8] = &[];
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon-disabled.svg") } else { "" };

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
                // Write the HTTP header
                stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/html\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(CONTENT_INDEX_HTML_GZ.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                // Write the actual data
                stream.write_all(&CONTENT_INDEX_HTML_GZ).await?;
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
                        // Write the HTTP header
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: ").await?;
                        stream.write_all(itoa::Buffer::new().format(CONTENT_PREVIEW_ICON_GZ.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        // Write the actual data
                        stream.write_all(CONTENT_PREVIEW_ICON_GZ).await?;
                    } else {                        //"/images/spinner.gif"
                        // Write the HTTP header
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: ").await?;
                        stream.write_all(itoa::Buffer::new().format(CONTENT_SPINNER_GIF.len()).as_bytes()).await?;
                        stream.write_all(b"\r\n\r\n").await?;
                        // Write the actual data
                        stream.write_all(&CONTENT_SPINNER_GIF).await?;
                }

            return Ok(());
        } else if path.starts_with("/we") {         //"/webworker.js"
                // Write the HTTP header
                stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(CONTENT_WEBWORKER_JS_GZ.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                // Write the actual data
                stream.write_all(&CONTENT_WEBWORKER_JS_GZ).await?; 
            return Ok(());
        } else if path.starts_with("/do") {
            if path.ends_with(".css") { //"/dot-viewer.css"
                    // Write the HTTP header
                    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/css\r\nContent-Length: ").await?;
                    stream.write_all(itoa::Buffer::new().format(CONTENT_DOT_VIEWER_CSS_GZ.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    // Write the actual data
                    stream.write_all(&CONTENT_DOT_VIEWER_CSS_GZ).await?;
              
            } else { //"/dot-viewer.js"               
                    // Write the HTTP header
                    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ").await?;
                    stream.write_all(itoa::Buffer::new().format(CONTENT_DOT_VIEWER_JS_GZ.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    // Write the actual data
                    stream.write_all(&CONTENT_DOT_VIEWER_JS_GZ).await?;               
            }
            return Ok(());
        } else if path.starts_with("/vi") {        //"/viz-lite.js"
            // Write the HTTP header
            stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ").await?;
            stream.write_all(itoa::Buffer::new().format(CONTENT_VIZ_LITE_GZ.len()).as_bytes()).await?;
            stream.write_all(b"\r\n\r\n").await?;
            // Write the actual data
            stream.write_all(&CONTENT_VIZ_LITE_GZ).await?;
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

#[cfg(test)]
mod meteric_server_tests {
    use async_std::test;


    // #[test]
    // pub(crate) async fn test_simple_process() {
    //     //1. build test graph, the input and output channels and our actor
    //     let mut graph = Graph::new_test(());
    //     let (tx_in, rx_in) = graph.channel_builder()
    //         .with_capacity(10).build();
    //     graph.actor_builder()
    //         .with_name("UnitTest")
    //         .build_spawn( move |context| internal_behavior(context, rx_in.clone(), None) );
    //
    //   // let d = DiagramData::NodeDef(0, ("".to_string(), "".to_string(), "".to_string()));
    //   //  let e = DiagramData::NodeProcessData(0, vec![]);
    //   //  let f = DiagramData::ChannelVolumeData(0, vec![]);
    //
    //
    //
    //     //2. add test data to the input channels
    //    // let test_data:Vec<DiagramData> = (0..1).map(|i| DiagramData { seq: i, data: vec![]  }).collect();
    //    // tx_in.testing_send(test_data, true).await;
    //
    //     tx_in.testing_close(Duration::from_millis(30)).await;
    //
    //     //3. run graph until the actor detects the input is closed
    //     graph.start_as_data_driven(Duration::from_secs(240));
    //
    // }

}
