use std::error::Error;
use std::process::{exit};
use futures::pin_mut;
use bytes::BytesMut;
use futures::select;
use log::*;
use tide::{Body, Request, Response, Server};
use tide::http::headers::CONTENT_ENCODING;
use tide::http::mime;
use crate::*;
use crate::dot::{build_dot, DotGraphFrames, MetricState, FrameHistory, apply_node_def, build_metric};
use crate::telemetry::metrics_collector::*;

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

    let mut app = tide::with_state(state.clone());
    add_all_telemetry_paths(&mut app);

    #[cfg(feature = "prometheus_metrics")]
        let _ = app.at("/metrics").get(|req: Request<Arc<Mutex<State>>>| async move {
        let body = Body::from_bytes({
            let state = req.state().lock().await;
            state.metric.clone()
        });
        let mut res = Response::new(200);
        res.set_content_type(mime::PLAIN);
        res.set_body(body);
        Ok(res)
    });

    #[cfg(any(feature = "telemetry_server_builtin", feature = "telemetry_server_cdn"))]
        let _ = app.at("/graph.dot").get(|req: Request<Arc<Mutex<State>>>| async move {
        let body = Body::from_bytes({
            let state = req.state().lock().await;
            state.doc.clone()
        });
        let mut res = Response::new(200);
        res.set_content_type(mime::PLAIN);
        res.set_body(body);
        Ok(res)
    });

    let server_handle = app.listen(format!("{}:{}", config::telemetry_server_ip(), config::telemetry_server_port()));
    pin_mut!(server_handle);

    while ctrl.is_running(&mut || rxg.is_empty() && rxg.is_closed()) {
        #[cfg(feature = "telemetry_on_telemetry")]
        ctrl.relay_stats_smartly();

        let a = server_handle.as_mut();
        let b = rx.wait_avail_units(2); // node data and edge data

        select! {
            e = a.fuse() => {
                warn!("Web server exited. {:?}", e);
                break;
            },
            _ = b.fuse() => {
                while let Some(msg) = ctrl.try_take(&mut rxg) {
                    match msg {
                        DiagramData::NodeDef(seq, defs) => {
                            let name = defs.0.name;
                            let id = defs.0.id;
                            apply_node_def(&mut metrics_state, name, id, defs.0, &defs.1, &defs.2, ctrl.frame_rate_ms);
                            metrics_state.seq = seq;
                            if config::TELEMETRY_HISTORY {
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

                            if config::TELEMETRY_HISTORY {
                                history.apply_edge(&total_take_send, ctrl.frame_rate_ms);
                                let flush_all = ctrl.is_liveliness_in(&[GraphLivelinessState::StopRequested, GraphLivelinessState::Stopped], true);

                                history.update(flush_all).await;
                                history.mark_position();
                            }

                            #[cfg(feature = "telemetry_on_telemetry")]
                            ctrl.relay_stats_smartly();

                            if rxg.is_empty() || frames.last_graph.elapsed().as_millis() > 2 * ctrl.frame_rate_ms as u128 {
                                build_dot(&metrics_state, rankdir, &mut frames.active_graph);
                                let graph_bytes = frames.active_graph.to_vec();
                                build_metric(&metrics_state, &mut frames.active_metric);
                                let metric_bytes = frames.active_metric.to_vec();

                                let mut state = state.lock().await;
                                state.doc = graph_bytes;
                                state.metric = metric_bytes;
                                frames.last_graph = Instant::now();
                            }
                        },
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(any(docsrs, feature = "telemetry_server_cdn", not(feature = "telemetry_server_builtin")))]
const CONTENT_VIZ_LITE_B64: &str = "";
#[cfg(all(not(any(docsrs, feature = "telemetry_server_cdn")), feature = "telemetry_server_builtin"))]
const CONTENT_VIZ_LITE_B64: &str = if config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/viz-lite.js.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_INDEX_HTML_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_INDEX_HTML_B64: &str = if config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/index.html.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_DOT_VIEWER_JS_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_DOT_VIEWER_JS_B64: &str = if config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/dot-viewer.js.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_DOT_VIEWER_CSS_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_DOT_VIEWER_CSS_B64: &str = if config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/dot-viewer.css.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_WEBWORKER_JS_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_WEBWORKER_JS_B64: &str = if config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/webworker.js.gz.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_SPINNER_GIF_B64: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_SPINNER_GIF_B64: &str = if config::TELEMETRY_SERVER { include_str!("../../target/static/telemetry/images/spinner.gif.b64") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_PREVIEW_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_PREVIEW_ICON_SVG: &str = if config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/preview-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = if config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/refresh-time-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_USER_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_USER_ICON_SVG: &str = if config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/user-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_IN_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_IN_ICON_SVG: &str = if config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = if config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-in-icon-disabled.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = if config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon.svg") } else { "" };

#[cfg(any(docsrs, not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))))]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = "";
#[cfg(all(not(docsrs), any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = if config::TELEMETRY_SERVER { include_str!("../../static/telemetry/images/zoom-out-icon-disabled.svg") } else { "" };

/// Adds all telemetry paths to the Tide server application.
///
/// # Parameters
/// - `app`: The Tide server application to which the paths are added.
fn add_all_telemetry_paths(app: &mut Server<Arc<Mutex<State>>>) {
    let _ = app.at("/webworker.js").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_WEBWORKER_JS_B64) {
            Ok(Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::JAVASCRIPT).build())
        } else {
            error!("Failed to decode CONTENT_WEBWORKER_JS_B64 {:?}", decode_base64(CONTENT_WEBWORKER_JS_B64));
            Ok(Response::builder(500).body(Body::from_string("Failed to decode CONTENT_WEBWORKER_JS_B64".to_string())).build())
        }
    });

    let _ = app.at("/dot-viewer.css").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_DOT_VIEWER_CSS_B64) {
            Ok(Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::CSS).build())
        } else {
            error!("Failed to decode CONTENT_DOT_VIEWER_CSS_B64 {:?}", decode_base64(CONTENT_DOT_VIEWER_CSS_B64));
            Ok(Response::builder(500).body(Body::from_string("Failed to decode CONTENT_DOT_VIEWER_CSS_B64".to_string())).build())
        }
    });

    let _ = app.at("/dot-viewer.js").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_DOT_VIEWER_JS_B64) {
            Ok(Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::JAVASCRIPT).build())
        } else {
            error!("Failed to decode CONTENT_DOT_VIEWER_JS_B64 {:?}", decode_base64(CONTENT_DOT_VIEWER_JS_B64));
            Ok(Response::builder(500).body(Body::from_string("Failed to decode CONTENT_DOT_VIEWER_JS_B64".to_string())).build())
        }
    });

    let _ = app.at("/viz-lite.js").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_VIZ_LITE_B64) {
            Ok(Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::JAVASCRIPT).build())
        } else {
            error!("Failed to decode CONTENT_VIZ_LITE_B64 {:?}", decode_base64(CONTENT_VIZ_LITE_B64));
            Ok(Response::builder(500).body(Body::from_string("Failed to decode CONTENT_VIZ_LITE_B64".to_string())).build())
        }
    });

    let _ = app.at("/").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_INDEX_HTML_B64) {
            Ok(Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::HTML).build())
        } else {
            error!("Failed to decode CONTENT_INDEX_HTML_B64 {:?}", decode_base64(CONTENT_INDEX_HTML_B64));
            Ok(Response::builder(500).body(Body::from_string("Failed to decode CONTENT_INDEX_HTML_B64".to_string())).build())
        }
    });

    let _ = app.at("/index.html").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_INDEX_HTML_B64) {
            Ok(Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::HTML).build())
        } else {
            error!("Failed to decode CONTENT_INDEX_HTML_B64 {:?}", decode_base64(CONTENT_INDEX_HTML_B64));
            Ok(Response::builder(500).body(Body::from_string("Failed to decode CONTENT_INDEX_HTML_B64".to_string())).build())
        }
    });

    let _ = app.at("/images/spinner.gif").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_SPINNER_GIF_B64) {
            Ok(Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .content_type("image/gif").build())
        } else {
            error!("Failed to decode CONTENT_SPINNER_GIF_B64 {:?}", decode_base64(CONTENT_SPINNER_GIF_B64));
            Ok(Response::builder(500).body(Body::from_string("Failed to decode CONTENT_SPINNER_GIF_B64".to_string())).build())
        }
    });

    let _ = app.at("/images/preview-icon.svg").get(|_| async move {
        Ok(Response::builder(200)
            .body(Body::from_bytes(CONTENT_PREVIEW_ICON_SVG.into()))
            .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/refresh-time-icon.svg").get(|_| async move {
        Ok(Response::builder(200)
            .body(Body::from_bytes(CONTENT_REFRESH_TIME_ICON_SVG.into()))
            .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/user-icon.svg").get(|_| async move {
        Ok(Response::builder(200)
            .body(Body::from_bytes(CONTENT_USER_ICON_SVG.into()))
            .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/zoom-in-icon.svg").get(|_| async move {
        Ok(Response::builder(200)
            .body(Body::from_bytes(CONTENT_ZOOM_IN_ICON_SVG.into()))
            .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/zoom-in-icon-disabled.svg").get(|_| async move {
        Ok(Response::builder(200)
            .body(Body::from_bytes(CONTENT_ZOOM_IN_ICON_DISABLED_SVG.into()))
            .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/zoom-out-icon.svg").get(|_| async move {
        Ok(Response::builder(200)
            .body(Body::from_bytes(CONTENT_ZOOM_OUT_ICON_SVG.into()))
            .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/zoom-out-icon-disabled.svg").get(|_| async move {
        Ok(Response::builder(200)
            .body(Body::from_bytes(CONTENT_ZOOM_OUT_ICON_DISABLED_SVG.into()))
            .content_type(mime::SVG).build())
    });
}

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

