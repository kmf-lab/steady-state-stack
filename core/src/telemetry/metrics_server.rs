use std::error::Error;
use std::net::{SocketAddr, TcpListener};
use std::pin::Pin;
use async_io::Async;
use bytes::BytesMut;
#[allow(unused_imports)]
use log::*;
use crate::*;
use crate::dot::{apply_node_def, build_dot, build_metric, Config, DotGraphFrames, FrameHistory, MetricState};
use crate::telemetry::metrics_collector::*;
use futures::io;
use futures::channel::oneshot::Receiver;
use std::io::Write;
use crate::commander_context::SteadyContext;

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
    
    //NOTE: we could use this to turn off the server if desired.
    let addr = Some(format!("{}:{}"
                            , steady_config::telemetry_server_ip()
                            , steady_config::telemetry_server_port()));

    let frame_rate_ms = context.frame_rate_ms;
    let ctrl = context;
    #[cfg(feature = "telemetry_on_telemetry")]
    let ctrl = ctrl.into_monitor([&rx], []);

    internal_behavior(ctrl, frame_rate_ms, rx, addr).await
}

async fn internal_behavior<C : SteadyCommander>(mut ctrl: C, frame_rate_ms: u64, rx: SteadyRx<DiagramData>, addr: Option<String>) -> Result<(), Box<dyn Error>> {



    let mut rxg = rx.lock().await;
    let mut metrics_state = MetricState::default();

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
    let config = Arc::new(Mutex::new(Config {
        rankdir: "LR".to_string()
        
        //TODO: context has our labels??? we must keep them here to flag our graph building.
        //TODO: start with the use logic since it must be fast and work back.
        
    }));

    let mut history = FrameHistory::new(frame_rate_ms);

    let (tcp_sender_tx, tcp_receiver_tx) = oneshot::channel();
    let tcp_receiver_tx_oneshot_shutdown = Arc::new(Mutex::new(tcp_receiver_tx));


    //Only spin up server if addr is provided, this allows for unit testing where we cannot open that port.
    if let Some(ref addr) = addr {

        let state2 = state.clone();
        let config2 = config.clone();


        let opt_tcp = bind_to_port(addr);
        if let Some(ref listener_new) = *opt_tcp {
            #[cfg(any(feature = "telemetry_server_builtin", feature = "telemetry_server_cdn"))]
            println!("Telemetry on http://{}", listener_new.local_addr().expect("Unable to get local address"));
            #[cfg(feature = "prometheus_metrics")]
            println!("Prometheus can scrape on on http://{}/metrics", listener_new.local_addr().expect("Unable to read local address"));
        }
        //NOTE: this is probably a mistake this loop could be its own actor.
        abstract_executor::spawn(async move {
            if let Some(ref listener_new) = *opt_tcp {
                handle_new_requests(tcp_receiver_tx_oneshot_shutdown, state2, config2, listener_new).await;
            }
        }).detach();


    }

    while ctrl.is_running(&mut || rxg.is_empty() && rxg.is_closed()) {
        //TODO: merge the above connection loop into this wait so we only have 1 thread and 1 loop.
        let _clean = await_for_all!( ctrl.wait_avail(&mut rxg,1) );

        if let Some(msg) = ctrl.try_take(&mut rxg) {
            process_msg(msg
                        , &mut metrics_state
                        , &mut history
                        , &mut frames
                        , frame_rate_ms
                        , ctrl.is_liveliness_in(&[GraphLivelinessState::StopRequested, GraphLivelinessState::Stopped])
                        , &rxg
                        , state.clone()
                        , config.clone()).await;
        }
    }
    let _ = tcp_sender_tx.send(());
    Ok(())
}


/// A trait combining `AsyncRead` and `AsyncWrite` for types that support both.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}

impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}

/// A trait for asynchronous listeners that can accept connections and provide their local address.
pub trait AsyncListener {
    /// Accepts a new connection asynchronously.
    ///
    /// Returns a future resolving to a stream implementing `AsyncReadWrite` and an optional socket address.
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output =std::io::Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>)>> + Send + 'a>>;

    /// Returns the local socket address of the listener.
    fn local_addr(&self) -> std::io::Result<SocketAddr>;
}

/// Implements `AsyncListener` for `Async<TcpListener>` to ensure true asynchronous acceptance.
impl AsyncListener for Async<TcpListener> {
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output =std::io::Result<(Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Option<SocketAddr>)>> + Send + 'a>> {
        Box::pin(async move {
            let (stream, addr) = self.accept().await?;
            Ok((Box::new(stream) as Box<dyn AsyncReadWrite + Send + Unpin + 'static>, Some(addr)))
        })
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.get_ref().local_addr()
    }
}

// ## Utility Functions

/// Binds a TCP listener to the specified address using `Async<TcpListener>`.
///
/// Returns an `Arc` containing the listener if successful, or `None` if binding fails.
pub fn bind_to_port(addr: &str) -> Arc<Option<Box<dyn AsyncListener + Send + Sync>>> {
    match TcpListener::bind(addr) {
        Ok(listener) => match Async::new(listener) {
            Ok(async_listener) => Arc::new(Some(Box::new(async_listener) as Box<dyn AsyncListener + Send + Sync>)),
            Err(e) => {
                error!("Unable to create async listener: {}", e);
                Arc::new(None)
            }
        },
        Err(e) => {
            error!("Unable to bind to http://{}: {}", addr, e);
            Arc::new(None)
        }
    }
}

/// Checks if an address can be bound to and returns the local address if successful.
pub(crate) fn check_addr(addr: &str) -> Option<String> {
    if let Ok(h) = TcpListener::bind(addr) {
        let local_addr = h.local_addr().expect("Unable to get local address");
        Some(format!("{}", local_addr))
    } else {
        None
    }
}

async fn handle_new_requests (
    tcp_receiver_tx_oneshot_shutdown: Arc<Mutex<Receiver<()>>>,
    state: Arc<Mutex<State>>,
    config: Arc<Mutex<Config>>,
    listener: &Box<dyn AsyncListener + Send + Sync>,
) {
    //NOTE: this server is fast but only does 1 request/response at a time. This is good enough
    //      for per/second metrics and many telemetry observers with slower refresh rates
    loop {
        let mut shutdown = tcp_receiver_tx_oneshot_shutdown.lock().await;
        select! {  _ = shutdown.deref_mut() => {  break;  },
                result = listener.accept().fuse() => {
                    match result {
                       Ok((stream, _peer_addr)) => {
                           let _ = handle_request(stream,state.clone(),config.clone()).await;
                       }
                       Err(e) => {
                           error!("Error accepting connection: {}",e);
                       }
                   }
           } }
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_msg(msg: DiagramData
                     , metrics_state: &mut MetricState
                     , history: &mut FrameHistory
                     , frames: &mut DotGraphFrames
                     , frame_rate_ms: u64
                     , flush_all: bool
                     , rxg: &MutexGuard<'_, Rx<DiagramData>>
                     , state: Arc<Mutex<State>>
                     , config: Arc<Mutex<Config>>

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
                if !metrics_state.nodes.is_empty() {
                    metrics_state.nodes[i].compute_and_refresh(*status, total_work_ns);
                }
            });
        },
        DiagramData::ChannelVolumeData(seq, total_take_send) => {
            total_take_send.iter().enumerate().for_each(|(i, (t, s))| {
                if !metrics_state.edges.is_empty() {
                   metrics_state.edges[i].compute_and_refresh(*s, *t);
                }
            });
            metrics_state.seq = seq;

            if steady_config::TELEMETRY_HISTORY {
                history.apply_edge(&total_take_send, frame_rate_ms);

                history.update(flush_all).await;
                history.mark_position();
            }

            if rxg.is_empty() || frames.last_graph.elapsed().as_millis() >= 2 * frame_rate_ms as u128 {

                let config = config.lock().await;
                build_dot(metrics_state, &mut frames.active_graph, &config);
                drop(config);
                
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

#[allow(dead_code)]
#[cfg(any(docsrs, feature = "telemetry_server_cdn", not(feature = "telemetry_server_builtin")))]
const CONTENT_VIZ_LITE_GZ: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(any(docsrs, feature = "telemetry_server_cdn")), feature = "telemetry_server_builtin"))]
const CONTENT_VIZ_LITE_GZ: & [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/viz-lite.js.gz") } else { &[] };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_INDEX_HTML_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_INDEX_HTML_GZ: & [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/index.html.gz") } else { &[] };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_DOT_VIEWER_JS_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_DOT_VIEWER_JS_GZ: & [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/dot-viewer.js.gz") } else { &[] };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_DOT_VIEWER_CSS_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_DOT_VIEWER_CSS_GZ: & [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/dot-viewer.css.gz") } else { &[] };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_WEBWORKER_JS_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_WEBWORKER_JS_GZ: & [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/webworker.js.gz") } else { &[] };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_SPINNER_GIF_B64: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_SPINNER_GIF: & [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../target/static/telemetry/images/spinner.gif") } else { &[] };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_PREVIEW_ICON_SVG: & [u8] = &[];
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_PREVIEW_ICON_GZ: & [u8] = if steady_config::TELEMETRY_SERVER { include_bytes!("../../static/telemetry/images/preview-icon.svg") } else { &[] };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/refresh-time-icon.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_USER_ICON_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_USER_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/user-icon.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_IN_ICON_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_IN_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon-disabled.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon.svg") } else { "" };

#[allow(dead_code)]
#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = "";
#[allow(dead_code)]
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = if steady_config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon-disabled.svg") } else { "" };

//   pub trait AsyncWriteExt: AsyncWrite   for the .read
//   pub trait AsyncReadExt: AsyncRead     for the .write_all

async fn handle_request<T>(mut stream: T,
                        state: Arc<Mutex<State>>,
                        _config: Arc<Mutex<Config>>) -> io::Result<()>
  where
    T: AsyncRead + AsyncWrite + Unpin
    {
    let mut buffer = vec![0; 1024];
    let _ = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer);

    // Parse the HTTP request to get the path.
    let path = request.split_whitespace().nth(1).unwrap_or("/");

    #[cfg(feature = "prometheus_metrics")]
    if path.starts_with("/me") { //for prometheus /metrics
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: ").await?;
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
        } else if path.starts_with("/set?") {//example /set?rankdir=LR&show=label1,label2&hide=label3,label4            
            let mut rankdir = "LR";
            //let mut show:Option<Split<&str>> = None; // TODO: Labels feature
            //let mut hide:Option<Split<&str>> = None; // TODO: Labels feature
            let mut parts = path.split("?");
            if let Some(_part) = parts.next() {
                if let Some(part) = parts.next() {
                    let parts = part.split("&");
                    for part in parts {
                        let mut parts = part.split("=");
                        if let Some(key) = parts.next() {
                            if let Some(value) = parts.next() {
                                if "rankdir"==key {
                                    rankdir = value;
                                }
                                // match key {
                                //     "rankdir" => rankdir = value,
                                //     //"show" => show = Some(value.split(",")),// TODO: Labels feature
                                //     //"hide" => hide = Some(value.split(",")),
                                //     _ => {}
                                // }
                            }
                        }
                    }
                }
            }
            if rankdir.eq("LR") || rankdir.eq("TB") {
                let mut c = _config.lock().await;
                c.rankdir = rankdir.to_string();
                // TODO: Labels feature
                // if c.apply_labels(show,hide) {
                //     stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await?;
                //     return Ok(());
                // }
            }
            stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n").await?;
            return Ok(());
        } else if path.eq("/") || path.starts_with("/in") || path.starts_with("/de") { //index
                // Write the HTTP header
                stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/html\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(CONTENT_INDEX_HTML_GZ.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                // Write the actual data
                stream.write_all(CONTENT_INDEX_HTML_GZ).await?;
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
                        stream.write_all(CONTENT_SPINNER_GIF).await?;
                }

            return Ok(());
        } else if path.starts_with("/we") {         //"/webworker.js"
                // Write the HTTP header
                stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ").await?;
                stream.write_all(itoa::Buffer::new().format(CONTENT_WEBWORKER_JS_GZ.len()).as_bytes()).await?;
                stream.write_all(b"\r\n\r\n").await?;
                // Write the actual data
                stream.write_all(CONTENT_WEBWORKER_JS_GZ).await?; 
            return Ok(());
        } else if path.starts_with("/do") {
            if path.ends_with(".css") { //"/dot-viewer.css"
                    // Write the HTTP header
                    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/css\r\nContent-Length: ").await?;
                    stream.write_all(itoa::Buffer::new().format(CONTENT_DOT_VIEWER_CSS_GZ.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    // Write the actual data
                    stream.write_all(CONTENT_DOT_VIEWER_CSS_GZ).await?;
              
            } else { //"/dot-viewer.js"               
                    // Write the HTTP header
                    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ").await?;
                    stream.write_all(itoa::Buffer::new().format(CONTENT_DOT_VIEWER_JS_GZ.len()).as_bytes()).await?;
                    stream.write_all(b"\r\n\r\n").await?;
                    // Write the actual data
                    stream.write_all(CONTENT_DOT_VIEWER_JS_GZ).await?;               
            }
            return Ok(());
        } else if path.starts_with("/vi") {        //"/viz-lite.js"
            // Write the HTTP header
            stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/javascript\r\nContent-Length: ").await?;
            stream.write_all(itoa::Buffer::new().format(CONTENT_VIZ_LITE_GZ.len()).as_bytes()).await?;
            stream.write_all(b"\r\n\r\n").await?;
            // Write the actual data
            stream.write_all(CONTENT_VIZ_LITE_GZ).await?;
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
    use std::sync::Arc;
    use std::time::Duration;
    use futures_timer::Delay;
    use crate::{ActorIdentity, GraphBuilder};
    use crate::monitor::ActorMetaData;
    use crate::telemetry::metrics_collector::DiagramData;
    use crate::telemetry::metrics_server::internal_behavior;

    #[async_std::test]    
    async fn test_simple() {
        let mut graph = GraphBuilder::for_testing().build(());
         
         let (tx_in, rx_in) = graph.channel_builder()
             .with_capacity(10).build();

        let rate_ms = graph.telemetry_production_rate_ms;

        graph.actor_builder()
            .with_name("UnitTest")
            .build_spawn( move |context| internal_behavior(context, rate_ms, rx_in.clone(), None) );
 
        let test_data:Vec<DiagramData> = (0..3).map(|i| DiagramData::NodeDef( i
                 , Box::new((
                    Arc::new(ActorMetaData{
                        ident: ActorIdentity::new(i as usize, "test_actor", None ),
                        ..Default::default() }), Box::new([]),Box::new([])
                ) ) )).collect();
        tx_in.testing_send_all(test_data, true).await;
      
        graph.start(); 
        Delay::new(Duration::from_millis(60)).await;
        graph.request_stop();
        graph.block_until_stopped(Duration::from_secs(15));
    
     }

}

#[cfg(test)]
mod http_telemetry_tests {
    use super::*;
    use crate::GraphBuilder;
    use std::time::Duration;
    use futures_timer::Delay;
    use isahc::ReadResponseExt;
    use crate::monitor::ActorStatus;

    #[async_std::test]
    async fn test_metrics_server() {
        if std::env::var("GITHUB_ACTIONS").is_err() {
            let (mut graph, server_ip, tx_in) = stand_up_test_server("127.0.0.1:0").await;

            // Step 5: Capture and validate the metrics server content
            // Fetch the metrics from the server
            if let Some(ref addr) = server_ip {
                print!(".");
                validate_path(&addr, Some("rankdir=LR"), "graph.dot");
                print!(".");
                validate_path(&addr, Some("font-family: sans-serif;"), "dot-viewer.css");
                print!(".");
                validate_path(&addr, Some("'1 sec': 1000,"), "dot-viewer.js");
                print!(".");
                validate_path(&addr, Some("this.importScripts('viz-lite.js');"), "webworker.js");
                print!(".");
                validate_path(&addr, Some("<title>Telemetry</title>"), "index.html");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "viz-lite.js");
                print!(".");
                #[cfg(feature = "prometheus_metrics")]
                validate_path(&addr, Some("="), "metric");
                print!(".");

                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/preview-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/refresh-time-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/user-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/zoom-in-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/zoom-in-icon-disabled.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/zoom-out-icon.svg");
                print!(".");
                #[cfg(feature = "telemetry_server_builtin")]
                validate_path(&addr, None, "images/zoom-out-icon-disabled.svg");
                print!(".");

                //TODO: new label feature, in progress 
                // validate_path(&addr, None, "set?rankdir=LR".into());
                // print!(".");

            } else {
                panic!("Telemetry address not available");
            }
            // Step 6: Stop the graph
            tx_in.testing_close(Duration::from_millis(10)).await;
            graph.request_stop();
            graph.block_until_stopped(Duration::from_secs(5));
        }
    }



    async fn stand_up_test_server(addr: &str) -> (Graph, Option<String>, LazySteadyTx<DiagramData>) {
        // Step 1: Set up a minimal graph
        let mut graph = GraphBuilder::for_testing()
            .with_telemtry_production_rate_ms(500)
            .build(());

        // Step 2: Start the metrics_server actor        
        let (tx_in, rx_in) = graph.channel_builder().build();
        if let Some(ref addr) = Some(addr.to_string()) {
            if let Some(addr) = check_addr(addr) {
                println!("{}",&addr);                
                launch_server(graph, Some(addr), tx_in, rx_in).await
            } else {
                panic!("Unable to Bind to http://{}", addr);
            }
        } else {
            panic!("Unable to Bind to http://{:?}", &addr);
        }        
    }

    async fn launch_server(mut graph: Graph, server_ip: Option<String>, tx_in: LazySteadyTx<DiagramData>
                           , rx_in: LazySteadyRx<DiagramData>) -> (Graph, Option<String>, LazySteadyTx<DiagramData>) {

        let server_ip_out = server_ip.clone();
        graph.actor_builder()
            .with_name("metrics_server")
            .build_spawn(move |context| {
                let frame_rate_ms = context.frame_rate_ms;
                internal_behavior(context, frame_rate_ms, rx_in.clone(), server_ip.clone())
            });

        // Step 3: Start the graph
        graph.start();

        // Allow the server to start
        Delay::new(Duration::from_millis(500)).await;

        // Step 4: Send test data to the metrics_server
        // Simulate DiagramData messages
        let sequence = 0;
        let mut data: Vec<DiagramData> = (0..4).map(|i| DiagramData::NodeDef(
            sequence,
            Box::new((
                Arc::new(ActorMetaData {
                    ident: ActorIdentity::new(i, "test_actor", None),
                    ..Default::default()
                }),
                Box::new([]),
                Box::new([]),
            )),
        )).collect();

        let node_status: Vec<ActorStatus> = (0..4).map(|_i|
            ActorStatus {
                await_total_ns: 100,
                unit_total_ns: 200,
                total_count_restarts: 1,
                iteration_start: 0,
                iteration_sum: 0,
                bool_stop: false,
                calls: [0; 6],
                thread_info: None,
            }
        ).collect();

        data.push(DiagramData::NodeProcessData(0, node_status.clone().into()));
        data.push(DiagramData::ChannelVolumeData(0, vec![(5, 10), (15, 20)].into()));
        data.push(DiagramData::NodeProcessData(1, node_status.into()));
        data.push(DiagramData::ChannelVolumeData(1, vec![(15, 20), (30, 30)].into()));

        tx_in.testing_send_all(data, false).await;
        (graph, server_ip_out, tx_in)
    }

    fn validate_path(addr: &&String, expected_text: Option<&str>, path: &str) {
        match isahc::get(format!("http://{}/{}", &addr, &path)) {
            Ok(mut response) => {
                assert_eq!(response.status(), 200);//, "got status {} from {}", response.status(),format!("http://{}/{}", &addr, &path));

                // Read and validate the response body
                let body = response.text().expect("Failed to read response body");
                //println!("Content:\n{}", body);

                if let Some(expected_text) = expected_text {
                    assert!(
                        body.contains(expected_text),
                        "not found {} in {}", expected_text,
                        body
                    );
                }
            },
            Err(_) => {
                info!("unable to test port: {}",&addr);
            }
        };
    }




}

/// Asynchronously writes data to a file, optionally flushing it.
///
/// Uses `spawn_blocking` to perform blocking file I/O in a separate thread.
pub(crate) async fn async_write_all(data: BytesMut, flush: bool, mut file: std::fs::File) -> std::io::Result<()> {
    spawn_blocking(move || {
        file.write_all(&data)?;
        if flush {
            file.flush()?;
        }
        Ok(())
    }).await
}