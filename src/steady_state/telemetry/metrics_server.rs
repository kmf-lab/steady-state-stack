use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use futures::lock::Mutex;
use crate::steady_state::config;
use crate::steady_state::telemetry::metrics_collector::DiagramData;
use crate::steady_state::dot::*;
use futures::FutureExt; // Provides the .fuse() method
use futures::pin_mut; // Add pin_mut

use bytes::BytesMut;
use futures::select;
use log::*;
use crate::steady_state::SteadyContext;
use tide::{Body, Request, Response, Server};
use tide::http::headers::CONTENT_ENCODING;
use tide::http::mime;
use crate::steady_state::Rx;
use crate::steady_state::GraphLivelinessState;
use crate::steady_state::telemetry::metrics_collector::DiagramData::{Edge, Node};


macro_rules! count_bits {
    ($num:expr) => {{
        let mut num = $num;
        let mut bits = 0;
        while num > 0 {
            num >>= 1;
            bits += 1;
        }
        bits
    }};
}

//we make the buckets for MA a power of 2 for easy rotate math
const MA_MIN_BUCKETS: usize = 1000/ config::TELEMETRY_PRODUCTION_RATE_MS;
const MA_BITS: usize = count_bits!(MA_MIN_BUCKETS);
const MA_ADJUSTED_BUCKETS:usize = 1 << MA_BITS;
const MA_BITMASK: usize = MA_ADJUSTED_BUCKETS - 1;

struct AppState {
    dot_graph: Arc<AtomicUsize>, // Stores a pointer to the current DOT graph
}

// websocat is a command line tool for connecting to WebSockets servers
// cargo install websocat
// websocat ws://127.0.0.1:8080/ws

//TODO:
//       stream send/take data on websocket
//       poll for history file (gap?) (send notice of flush)





#[derive(Clone)]
struct State {
    doc: Vec<u8>,
}

//TODO: add the rest of these files and serve them
//TODO: for each of these store the zipped version in the repo instead and serve that is possible or unzip for the client.
//      this will add < 500K to the binary size and will make the server much faster

pub(crate) async fn run(monitor: SteadyContext
                        , rx: Arc<Mutex<Rx<DiagramData>>>) -> std::result::Result<(),()> {


    let mut history = FrameHistory::new().await;

    let mut rx_guard = rx.lock().await;
    let rx = rx_guard.deref_mut();

    //     monitor.init_stats(&[&rx], &[]); //TODO: this is not needed for this telemetry
    let mut dot_state = DotState {
        seq: u64::MAX,
        nodes: Vec::new(), //position is the telemetry id
        edges: Vec::new(), //position is the channel id
    };

    let top_down = false;
    let rankdir = if top_down { "TB" } else { "LR" };
    let mut frames = DotGraphFrames {
        active_graph: BytesMut::new(),
    };

// Define a new instance of the state.
    let state = Arc::new(Mutex::new(State {
        doc: Vec::new()
    }));

    let mut app = tide::with_state(state.clone());

    add_all_telemetry_paths(&mut app);



    // TODO: pick up our history file, add the current buffer and send
    //       we may want to zip it first for faster transfer
    //let _ = app.at("/history.dat")

    let _ = app.at("/graph.dot")
        .get(|req: Request<Arc<Mutex<State>>>| async move {
            let body = Body::from_bytes({
                let guard = req.state().lock().await;
                let state = guard.deref();
                state.doc.clone()
            });
            let mut res = Response::new(200);
            res.set_content_type(mime::PLAIN);  // Set the content type to text/plain
            res.set_body(body);
            Ok(res)
        });

    let server_handle = app.listen("127.0.0.1:8080");
    pin_mut!(server_handle);

    let runtime_state = monitor.runtime_state.clone();


    loop {
        select! {
            _ = server_handle.as_mut().fuse() => {
                println!("Web server exited.");
                break;
            },
            msg = rx.take_async().fuse() => {
                  match msg {
                         Ok(Node(seq, name, id, channels_in, channels_out)) => {

                              refresh_structure(&mut dot_state
                                               , name
                                               , id
                                               , channels_in.clone()
                                               , channels_out.clone()
                              );

                              dot_state.seq = seq;
                              if config::TELEMETRY_HISTORY  {
                                        history.apply_node(name, id, channels_in.clone(), channels_out.clone());
                              }
                         },
                         Ok(Edge(seq
                                 , total_take
                                 , total_send)) => {
                              //note on init we may not have the same length...
                              assert_eq!(total_take.len(), total_send.len());
                              total_send.iter()
                                        .zip(total_take.iter())
                                        .enumerate()
                                        .for_each(|(i,(s,t))| dot_state.edges[i].compute_and_refresh(*s,*t));

                              dot_state.seq = seq;
                              //NOTE: generate the new graph
                              frames.active_graph.clear(); // Clear the buffer for reuse
                              build_dot(&mut dot_state, rankdir, &mut frames.active_graph);
                              let vec = frames.active_graph.to_vec();
                                { //block to ensure we drop the guard quckly
                                    let mut state_guard = state.lock().await;
                                    state_guard.deref_mut().doc = vec;
                                }

                            if config::TELEMETRY_HISTORY  {
                               //TODO: add assert that this only happens once every 32ms


                                  //NOTE we do not expect to get any more messages for this seq
                                  //    and we have 32ms or so to record the history log file
                                  history.apply_edge(total_take, total_send);

                                  //since we got the edge data we know we have a full frame
                                  //and we can update the history

                            let flush_all:bool = {
                                 let runtime_guard = runtime_state.lock().await;
                                 let state = runtime_guard.deref();
                                   state == &GraphLivelinessState::StopInProgress
                                || state == &GraphLivelinessState::StopRequested
                                || state == &GraphLivelinessState::Stopped
                            };

                            history.update(dot_state.seq,flush_all).await;

                                  //must mark this for next time
                                  history.mark_position();
                            }


                         },

                        Err(msg) => {error!("Unexpected error on incoming message: {}",msg)}
                  }

            }
        }
    }

    Ok(())
}


const CONTENT_VIZ_LITE_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/viz-lite.js.gz.b64")} else {""};
const CONTENT_INDEX_HTML_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/index.html.gz.b64")} else {""};
const CONTENT_DOT_VIEWER_JS_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/dot-viewer.js.gz.b64")} else {""};
const CONTENT_DOT_VIEWER_CSS_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/dot-viewer.css.gz.b64")} else {""};
const CONTENT_WEBWORKER_JS_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/webworker.js.gz.b64")} else {""};
const CONTENT_SPINNER_GIF_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/images/spinner.gif.b64")} else {""};

const CONTENT_PREVIEW_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/images/preview-icon.svg")} else {""};
const CONTENT_REFRESH_TIME_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/images/refresh-time-icon.svg")} else {""};
const CONTENT_USER_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/images/user-icon.svg")} else {""};
const CONTENT_ZOOM_IN_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/images/zoom-in-icon.svg")} else {""};
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/images/zoom-in-icon-disabled.svg")} else {""};
const CONTENT_ZOOM_OUT_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/images/zoom-out-icon.svg")} else {""};
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/images/zoom-out-icon-disabled.svg")} else {""};



fn add_all_telemetry_paths(app: &mut Server<Arc<Mutex<State>>>) {
   //NOTE: this could be be better with quic but that is not supported by tide yet
    // further this could be done by a macro
    let _ = app.at("/webworker.js").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_WEBWORKER_JS_B64) {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::JAVASCRIPT).build())
        } else {
            error!("Failed to decode CONTENT_WEBWORKER_JS_B64 {:?} ",decode_base64(CONTENT_WEBWORKER_JS_B64));
            Ok(tide::Response::builder(500).body(Body::from_string("Failed to decode CONTENT_WEBWORKER_JS_B64".to_string())).build())
        }
    });

    let _ = app.at("/dot-viewer.css").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_DOT_VIEWER_CSS_B64) {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::CSS).build())
        } else {
            error!("Failed to decode CONTENT_DOT_VIEWER_CSS_B64 {:?} ",decode_base64(CONTENT_DOT_VIEWER_CSS_B64));
            Ok(tide::Response::builder(500).body(Body::from_string("Failed to decode CONTENT_DOT_VIEWER_CSS_B64".to_string())).build())
        }
    });

    let _ = app.at("/dot-viewer.js").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_DOT_VIEWER_JS_B64) {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::JAVASCRIPT).build())
        } else {
            error!("Failed to decode CONTENT_DOT_VIEWER_JS_B64 {:?} ",decode_base64(CONTENT_DOT_VIEWER_JS_B64));
            Ok(tide::Response::builder(500).body(Body::from_string("Failed to decode CONTENT_DOT_VIEWER_JS_B64".to_string())).build())
        }
    });


    let _ = app.at("/viz-lite.js").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_VIZ_LITE_B64) {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::JAVASCRIPT).build())
        } else {
            error!("Failed to decode CONTENT_VIZ_LITE_B64 {:?} ",decode_base64(CONTENT_VIZ_LITE_B64));
            Ok(tide::Response::builder(500).body(Body::from_string("Failed to decode CONTENT_VIZ_LITE_B64".to_string())).build())
        }
    });

    let _ = app.at("/").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_INDEX_HTML_B64) {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::HTML).build())
        } else {
            error!("Failed to decode CONTENT_INDEX_HTML_B64 {:?} ",decode_base64(CONTENT_INDEX_HTML_B64));
            Ok(tide::Response::builder(500).body(Body::from_string("Failed to decode CONTENT_INDEX_HTML_B64".to_string())).build())
        }
    });

    let _ = app.at("/index.html").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_INDEX_HTML_B64) {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::HTML).build())
        } else {
            error!("Failed to decode CONTENT_INDEX_HTML_B64 {:?} ",decode_base64(CONTENT_INDEX_HTML_B64));
            Ok(tide::Response::builder(500).body(Body::from_string("Failed to decode CONTENT_INDEX_HTML_B64".to_string())).build())
        }
    });


    let _ = app.at("/images/spinner.gif").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_SPINNER_GIF_B64) {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .content_type("image/gif").build())
        } else {
            error!("Failed to decode CONTENT_SPINNER_GIF_B64 {:?} ",decode_base64(CONTENT_SPINNER_GIF_B64));
            Ok(tide::Response::builder(500).body(Body::from_string("Failed to decode CONTENT_SPINNER_GIF_B64".to_string())).build())
        }
    });

    let _ = app.at("/images/preview-icon.svg").get(|_| async move {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(CONTENT_PREVIEW_ICON_SVG.into()))
                .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/refresh-time-icon.svg").get(|_| async move {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(CONTENT_REFRESH_TIME_ICON_SVG.into()))
                .content_type(mime::SVG).build())
    });


    let _ = app.at("/images/user-icon.svg").get(|_| async move {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(CONTENT_USER_ICON_SVG.into()))
                .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/zoom-in-icon.svg").get(|_| async move {

            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(CONTENT_ZOOM_IN_ICON_SVG.into()))
                .content_type(mime::SVG).build())

    });

    let _ = app.at("/images/zoom-in-icon-disabled.svg").get(|_| async move {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(CONTENT_ZOOM_IN_ICON_DISABLED_SVG.into()))
                .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/zoom-out-icon.svg").get(|_| async move {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(CONTENT_ZOOM_OUT_ICON_SVG.into()))
                .content_type(mime::SVG).build())
    });

    let _ = app.at("/images/zoom-out-icon-disabled.svg").get(|_| async move {
            Ok(tide::Response::builder(200)
                .body(Body::from_bytes(CONTENT_ZOOM_OUT_ICON_DISABLED_SVG.into()))
                .content_type(mime::SVG).build())
    });


}



///
/// I would rather use the base64 crate but it is very broken and unusable at this time. Please fix me.
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
            x => return Err(format!("Invalid character {:?} in Base64 input.",x)),
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


