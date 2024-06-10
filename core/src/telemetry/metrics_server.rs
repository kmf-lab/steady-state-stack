use std::error::Error;
use std::process::{exit};

use futures::pin_mut; // Add pin_mut

use bytes::BytesMut;
use futures::select;
use log::*;
use tide::{Body, Request, Response, Server};
use tide::http::headers::CONTENT_ENCODING;
use tide::http::mime;

use crate::*;
use crate::dot::{build_dot, DotGraphFrames, MetricState, FrameHistory, apply_node_def, build_metric};
use crate::telemetry::metrics_collector::*;


// websocat is a command line tool for connecting to WebSockets servers
// cargo install websocat
// websocat ws://127.0.0.1:8080/ws

//TODO: 2025 history display stream send/take data on websocket
//          poll for history file (gap?) (send notice of flush)



pub const NAME: &str = "metrics_server";

#[derive(Clone)]
struct State {
    doc: Vec<u8>,
    metric: Vec<u8>,
}

pub(crate) async fn run(context: SteadyContext
                        , rx: SteadyRx<DiagramData>) -> std::result::Result<(),Box<dyn Error>> {

    let ctrl = context;
    #[cfg(feature = "telemetry_on_telemetry")]
    let mut ctrl =into_monitor!(ctrl, [rx], []);

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
    let mut history = FrameHistory::new();

    let mut app = tide::with_state(state.clone());

    add_all_telemetry_paths(&mut app);

    // TODO: pick up our history file, add the current buffer and send
    //       we may want to zip it first for faster transfer
    // let _ = app.at("/history.dat")
    //     .get(|_| async move {
    //         let body = Body::from_bytes(vec![]); //TODO: finish.
    //         let mut res = Response::new(200);
    //         res.set_content_type(mime::PLAIN);  // Set the content type to text/plain
    //         res.set_body(body);
    //         Ok(res)
    //     });
    // TODO: build a small CLI app which plays it into a a server
    //       add some text commands to pause requind etc

    #[cfg(any(feature = "prometheus_metrics") )]
    let _ = app.at("/metrics")
        .get(|req: Request<Arc<Mutex<State>>>| async move {
            let body = Body::from_bytes({
                let state = req.state().lock().await;
                state.metric.clone()
            });
            let mut res = Response::new(200);
            res.set_content_type(mime::PLAIN);  // Set the content type to text/plain
            res.set_body(body);
            Ok(res)
            });

    #[cfg(any(feature = "telemetry_server_builtin", feature = "telemetry_server_cdn") )]
    let _ = app.at("/graph.dot")
        .get(|req: Request<Arc<Mutex<State>>>| async move {
            let body = Body::from_bytes({
                let state = req.state().lock().await;
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


  //  trace!("server is up and running");
    while ctrl.is_running(&mut || { rxg.is_empty() && rxg.is_closed()  } )  {

        #[cfg(feature = "telemetry_on_telemetry")]
        ctrl.relay_stats_smartly();

        let a = server_handle.as_mut();
        let b = rx.wait_avail_units(2); //node data and edge data

        select! {
            e = a.fuse() => {
                warn!("Web server exited. {:?}",e);
                break;
            },
            _ = b.fuse() => {

                //we may have more than 2 so take them all if we can while we are here
               while let Some(msg) = ctrl.try_take(&mut rxg)

                   {

                      match msg {
                             //define node in the graph
                             DiagramData::NodeDef(seq, defs) => {
                                    //let (actor, channels_in, channels_out) = *defs;
                                    //confirm the data we get from the collector
                                    //warn!("new node {:?} {:?} in {:?} out {:?}",actor.id,actor.name,channels_in.len(),channels_out.len());
                                    //channels_in.iter().for_each(|x| warn!("          channel in {:?} to {:?}",x.id,actor.id));
                                    //channels_out.iter().for_each(|x| warn!("         channel out {:?} from {:?}",x.id,actor.id));


                                    let name = defs.0.name;
                                    let id = defs.0.id;
                                    apply_node_def(&mut metrics_state
                                                       , name
                                                       , id
                                                       , defs.0
                                                       , &defs.1
                                                       , &defs.2
                                                       , ctrl.frame_rate_ms
                                    );

                                    metrics_state.seq = seq;
                                    if config::TELEMETRY_HISTORY  {
                                          history.apply_node(name, id
                                                          , &defs.1
                                                          , &defs.2);
                                    }
                             },
                             //Node cpu usage
                             DiagramData::NodeProcessData(seq, actor_status) => {
                                    //assert_eq!(seq, dot_state.seq);

                                    //sum up all actor work so we can find the percentage of each
                                    let total_work_ns:u128 = actor_status
                                                .iter()
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
                                     if metrics_state.nodes.is_empty() { exit(-1); }

                                     assert!(!metrics_state.nodes.is_empty());

                                     metrics_state.nodes[i].compute_and_refresh(*status, total_work_ns);
                                });
                             },
                             //Channel usage
                             DiagramData::ChannelVolumeData(seq
                                     , total_take_send) => {

                                  total_take_send.iter()
                                            .enumerate()
                                            .for_each(|(i,(t,s))| {
                                            // we are in a bad state just exit and give up
                                            #[cfg(debug_assertions)]
                                            if metrics_state.edges.is_empty() { exit(-1); }

                                            assert!(!metrics_state.edges.is_empty());
                                            metrics_state.edges[i].compute_and_refresh(*s,*t);
                                     });
                                  metrics_state.seq = seq;
    //TODO: the history file also needs the RATE so we know the span it covers.

                              //if history is on we may never skip it since it is our log
                                if config::TELEMETRY_HISTORY  {
                                    //  info!("write hitory file");
                                      //NOTE we do not expect to get any more messages for this seq
                                      //    and we have 32ms or so to record the history log file
                                      history.apply_edge(&total_take_send, ctrl.frame_rate_ms as u64);

                                      //since we got the edge data we know we have a full frame
                                      //and we can update the history

                                      let flush_all:bool = ctrl.is_liveliness_in(&[GraphLivelinessState::StopRequested,GraphLivelinessState::Stopped],true);

                                      history.update(metrics_state.seq,flush_all).await;

                                      //must mark this for next time
                                      history.mark_position();
                                   //  info!("done write hitory file");

                                }

                                #[cfg(feature = "telemetry_on_telemetry")]
                                ctrl.relay_stats_smartly();


                              if rxg.is_empty()
                                || frames.last_graph.elapsed().as_millis() > 2* ctrl.frame_rate_ms as u128
                                 {
                                     
                                      //only build dot if we have valid data at this time
                                      build_dot(&metrics_state, rankdir, &mut frames.active_graph);
                                      let graph_bytes = frames.active_graph.to_vec();
                                                                          
                                      build_metric(&metrics_state, &mut frames.active_metric);
                                      let metric_bytes = frames.active_metric.to_vec();
                                     
                                      { //block to ensure we drop the guard quickly
                                            let mut state = state.lock().await;
                                            state.doc = graph_bytes;
                                            state.metric = metric_bytes;
                                      }
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

// when building docs we must not bring in the web resources since they will not be present
// when building for cdn we do not load the viz-lite into the binary to keep it smaller
#[cfg(any(docsrs, feature = "telemetry_server_cdn", not( feature = "telemetry_server_builtin" )))]
const CONTENT_VIZ_LITE_B64: &str = "";
#[cfg(all(not(any(docsrs, feature = "telemetry_server_cdn")), feature = "telemetry_server_builtin" )  )]
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
            Ok(Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .header(CONTENT_ENCODING, "gzip")
                .content_type(mime::JAVASCRIPT).build())
        } else {
            error!("Failed to decode CONTENT_WEBWORKER_JS_B64 {:?} ",decode_base64(CONTENT_WEBWORKER_JS_B64));
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
            error!("Failed to decode CONTENT_DOT_VIEWER_CSS_B64 {:?} ",decode_base64(CONTENT_DOT_VIEWER_CSS_B64));
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
            error!("Failed to decode CONTENT_DOT_VIEWER_JS_B64 {:?} ",decode_base64(CONTENT_DOT_VIEWER_JS_B64));
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
            error!("Failed to decode CONTENT_VIZ_LITE_B64 {:?} ",decode_base64(CONTENT_VIZ_LITE_B64));
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
            error!("Failed to decode CONTENT_INDEX_HTML_B64 {:?} ",decode_base64(CONTENT_INDEX_HTML_B64));
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
            error!("Failed to decode CONTENT_INDEX_HTML_B64 {:?} ",decode_base64(CONTENT_INDEX_HTML_B64));
            Ok(Response::builder(500).body(Body::from_string("Failed to decode CONTENT_INDEX_HTML_B64".to_string())).build())
        }
    });


    let _ = app.at("/images/spinner.gif").get(|_| async move {
        if let Ok(bytes) = decode_base64(CONTENT_SPINNER_GIF_B64) {
            Ok(Response::builder(200)
                .body(Body::from_bytes(bytes.clone()))
                .content_type("image/gif").build())
        } else {
            error!("Failed to decode CONTENT_SPINNER_GIF_B64 {:?} ",decode_base64(CONTENT_SPINNER_GIF_B64));
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


