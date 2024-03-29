use std::error::Error;
use std::ops::{Deref, DerefMut};
use std::process::{exit};
use std::sync::Arc;
use futures::lock::Mutex;
use futures::FutureExt; // Provides the .fuse() method
use futures::pin_mut; // Add pin_mut

use bytes::BytesMut;
use futures::select;
use log::*;
use tide::{Body, Request, Response, Server};
use tide::http::headers::CONTENT_ENCODING;
use tide::http::mime;
use crate::*;
use crate::dot::{build_dot, DotGraphFrames, DotState, FrameHistory, refresh_structure};
use crate::telemetry::metrics_collector::*;





// websocat is a command line tool for connecting to WebSockets servers
// cargo install websocat
// websocat ws://127.0.0.1:8080/ws

//TODO:
//       stream send/take data on websocket
//       poll for history file (gap?) (send notice of flush)



pub const NAME: &str = "metrics_server";

#[derive(Clone)]
struct State {
    doc: Vec<u8>,
}

pub(crate) async fn run(context: SteadyContext
                        , rx: SteadyRx<DiagramData>) -> std::result::Result<(),Box<dyn Error>> {

    let liveliness = context.liveliness();

    let mut optional_monitor:Option<_> = None;
    let mut optional_context:Option<_> = None;

    if config::SHOW_TELEMETRY_ON_TELEMETRY {
        optional_context = None;
        optional_monitor = Some(context.into_monitor([&rx], []));
    } else {
        optional_monitor = None;
        optional_context = Some(context);
    };



    let mut rx_guard = rx.lock().await;
    let rx = rx_guard.deref_mut();

    let mut dot_state = DotState::default();

    let top_down = false;
    let rankdir = if top_down { "TB" } else { "LR" };
    let mut frames = DotGraphFrames {
        last_graph: Instant::now(),
        active_graph: BytesMut::new(),
    };

// Define a new instance of the state.
    let state = Arc::new(Mutex::new(State {
        doc: Vec::new()
    }));
    let mut history = FrameHistory::new();

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

    // NOTE: this must be done after we defined all our routes
    let server_handle = app.listen(format!("{}:{}"
                                           ,config::telemetry_server_ip()
                                           ,config::telemetry_server_port()));

    // This future must be pinned before awaiting it to ensure it remains fixed in memory,
    // as it might contain self-references or internal state that requires stable addresses.
    // Pinning is crucial for safely using futures in async control flow, like select!.

    pin_mut!(server_handle);

    loop {

        if let Some(ref mut monitor) = optional_monitor {
            monitor.relay_stats_smartly().await;
            if !monitor.is_running(&mut || rx.is_empty() && rx.is_closed() ){
                break;
            }
        } else {
            if let Some(ref mut context) = optional_context {
                if !context.is_running(&mut || rx.is_empty() && rx.is_closed() ) {
                    break;
                }
            }
        }

        select! {
            _ = server_handle.as_mut().fuse() => {
                warn!("Web server exited.");
                break;
            },
            mut msg = rx.take_async().fuse() => {

                 //stay and process as much as we can
                 loop {
                        if let Some(ref mut monitor) = optional_monitor {
                            monitor.relay_stats_smartly().await; //TODO: if this is not done must detect and give helpful warning.
                        }

                      match msg {
                             Some(DiagramData::NodeDef(seq, defs)) => {
                                // these are all immutable constants for the life of the node

                                    let (actor, channels_in, channels_out) = *defs;

                                    let name = actor.name;
                                    let id = actor.id;
                                    refresh_structure(&mut dot_state
                                                       , name
                                                       , id
                                                       , actor
                                                       , &channels_in
                                                       , &channels_out
                                      );

                                      dot_state.seq = seq;
                                      if config::TELEMETRY_HISTORY  {
                                          history.apply_node(name, id
                                                          , &channels_in
                                                          , &channels_out);
                                      }

                             },
                             Some(DiagramData::NodeProcessData(_,actor_status)) => {

                                //sum up all actor work so we can find the percentage of each
                                let total_work_ns:u128 = actor_status.iter()
                                            .map(|status| {
                                                assert!(status.unit_total_ns>=status.await_total_ns, "unit_total_ns:{} await_total_ns:{}",status.unit_total_ns,status.await_total_ns);
                                                (status.unit_total_ns-status.await_total_ns) as u128
                                            })
                                            .sum();


                                //process each actor status
                                actor_status.iter()
                                            .enumerate()
                                            .for_each(|(i, status)| {

                                     // we are in a bad state just exit and give up
                                    #[cfg(debug_assertions)]
                                    if dot_state.nodes.is_empty() { exit(-1); }
                                    assert!(!dot_state.nodes.is_empty());

                                    dot_state.nodes[i].compute_and_refresh(*status, total_work_ns);
                                });
                             },
                             Some(DiagramData::ChannelVolumeData(seq
                                     , total_take_send)) => {

                                  // trace!("new data {:?} ",total_take_send);

                                  total_take_send.iter()
                                            .enumerate()
                                            .for_each(|(i,(t,s))| {

                                            // we are in a bad state just exit and give up
                                            #[cfg(debug_assertions)]
                                            if dot_state.edges.is_empty() { exit(-1); }
                                            assert!(!dot_state.edges.is_empty());

                                        dot_state.edges[i].compute_and_refresh(*s,*t);
                                     });
                                  dot_state.seq = seq;

                              //if history is on we may never skip it since it is our log
                                if config::TELEMETRY_HISTORY  {

                                      //NOTE we do not expect to get any more messages for this seq
                                      //    and we have 32ms or so to record the history log file
                                      history.apply_edge(&total_take_send);

                                      //since we got the edge data we know we have a full frame
                                      //and we can update the history

                                  //TODO: we can not stop early we need to know that the other
                                  //      actors have not raised objections to the shutdown.
                                    let flush_all:bool =
                                        if let Ok(lock) = liveliness.try_read() {
                                            lock.state == GraphLivelinessState::StopRequested
                                            || lock.state == GraphLivelinessState::Stopped
                                        } else {
                                            //unable to lock so be safe and flush
                                            true
                                        };


                                      history.update(dot_state.seq,flush_all).await;

                                      //must mark this for next time
                                      history.mark_position();
                                }

                               //visual graph is after history to give more time
                               //for more frames to arrive

                              if rx.is_empty()
                                || frames.last_graph.elapsed().as_millis() > 2*config::TELEMETRY_PRODUCTION_RATE_MS as u128
                                 {
                                      //NOTE: generate the new graph, this is costly so we
                                      //      skip it if we have more data to process
                                      frames.active_graph.clear(); // Clear the buffer for reuse
                                      build_dot(&dot_state, rankdir, &mut frames.active_graph);
                                      let vec = frames.active_graph.to_vec();
                                      { //block to ensure we drop the guard quickly
                                            let mut state_guard = state.lock().await;
                                            state_guard.deref_mut().doc = vec;
                                      }
                                      frames.last_graph = Instant::now();
                                }

                             },

                            None => {error!("Unexpected error")}
                      }
                      //if we an consume the rest without async or blocking do so
                      if let Some(new_msg) = rx.try_take() {
                        msg = Some(new_msg);
                        continue;
                      } else {
                         break;
                      }

                  }

            }
        }
    }

    Ok(())
}

#[cfg(docsrs)]
const CONTENT_VIZ_LITE_B64: &str = "";
#[cfg(not(docsrs))]
const CONTENT_VIZ_LITE_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../target/static/telemetry/viz-lite.js.gz.b64")} else {""};

#[cfg(docsrs)]
const CONTENT_INDEX_HTML_B64: &str = "";
#[cfg(not(docsrs))]
const CONTENT_INDEX_HTML_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../target/static/telemetry/index.html.gz.b64")} else {""};

#[cfg(docsrs)]
const CONTENT_DOT_VIEWER_JS_B64: &str = "";
#[cfg(not(docsrs))]
const CONTENT_DOT_VIEWER_JS_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../target/static/telemetry/dot-viewer.js.gz.b64")} else {""};

#[cfg(docsrs)]
const CONTENT_DOT_VIEWER_CSS_B64: &str = "";
#[cfg(not(docsrs))]
const CONTENT_DOT_VIEWER_CSS_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../target/static/telemetry/dot-viewer.css.gz.b64")} else {""};

#[cfg(docsrs)]
const CONTENT_WEBWORKER_JS_B64: &str = "";
#[cfg(not(docsrs))]
const CONTENT_WEBWORKER_JS_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../target/static/telemetry/webworker.js.gz.b64")} else {""};


#[cfg(docsrs)]
const CONTENT_SPINNER_GIF_B64: &str = "";
#[cfg(not(docsrs))]
const CONTENT_SPINNER_GIF_B64: &str = if config::TELEMETRY_SERVER {include_str!("../../target/static/telemetry/images/spinner.gif.b64")} else {""};

#[cfg(docsrs)]
const CONTENT_PREVIEW_ICON_SVG: &str = "";
#[cfg(not(docsrs))]
const CONTENT_PREVIEW_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../static/telemetry/images/preview-icon.svg")} else {""};

#[cfg(docsrs)]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = "";
#[cfg(not(docsrs))]
const CONTENT_REFRESH_TIME_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../static/telemetry/images/refresh-time-icon.svg")} else {""};

#[cfg(docsrs)]
const CONTENT_USER_ICON_SVG: &str = "";
#[cfg(not(docsrs))]
const CONTENT_USER_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../static/telemetry/images/user-icon.svg")} else {""};

#[cfg(docsrs)]
const CONTENT_ZOOM_IN_ICON_SVG: &str = "";
#[cfg(not(docsrs))]
const CONTENT_ZOOM_IN_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../static/telemetry/images/zoom-in-icon.svg")} else {""};

#[cfg(docsrs)]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = "";
#[cfg(not(docsrs))]
const CONTENT_ZOOM_IN_ICON_DISABLED_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../static/telemetry/images/zoom-in-icon-disabled.svg")} else {""};

#[cfg(docsrs)]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = "";
#[cfg(not(docsrs))]
const CONTENT_ZOOM_OUT_ICON_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../static/telemetry/images/zoom-out-icon.svg")} else {""};

#[cfg(docsrs)]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = "";
#[cfg(not(docsrs))]
const CONTENT_ZOOM_OUT_ICON_DISABLED_SVG: &str = if config::TELEMETRY_SERVER {include_str!("../../static/telemetry/images/zoom-out-icon-disabled.svg")} else {""};



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


