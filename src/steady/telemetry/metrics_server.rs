use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use futures::lock::Mutex;
use crate::steady::config;
use crate::steady::telemetry::metrics_collector::DiagramData;
use crate::steady::dot::*;
use futures::FutureExt; // Provides the .fuse() method
use futures::pin_mut; // Add pin_mut

use bytes::BytesMut;
use futures::select;
use log::*;
use crate::steady::monitor::SteadyMonitor;
use tide::{Body, Request, Response};
use tide::http::mime;
use crate::steady::channel::SteadyRx;
use crate::steady::graph::GraphRuntimeState;
use crate::steady::telemetry::metrics_collector::DiagramData::{Edge, Node};


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

const CONTENT_INDEX_HTML: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/index.html")} else {""};
// gzip -c ../../../static/telemetry/viz-lite.js > viz-lite.js.gz
const CONTENT_VIZ_LITE: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/viz-lite.js")} else {""};
const CONTENT_DOT_VIEWER_JS: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/dot-viewer.js")} else {""};
const CONTENT_DOT_VIEWER_CSS: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/dot-viewer.css")} else {""};
const CONTENT_WEBWORKER_JS: &str = if config::TELEMETRY_SERVER {include_str!("../../../static/telemetry/webworker.js")} else {""};

//TODO: add the rest of these files and serve them
//TODO: for each of these store the zipped version in the repo instead and serve that is possible or unzip for the client.
//      this will add < 500K to the binary size and will make the server much faster

pub(crate) async fn run(monitor: SteadyMonitor
                        , rx: Arc<Mutex<SteadyRx<DiagramData>>>) -> std::result::Result<(),()> {


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
    let mut state = Arc::new(Mutex::new(State {
        doc: Vec::new()
    }));

    let mut app = tide::with_state(state.clone());

    let _ = app.at("/").serve_file("static/telemetry/index.html");
    let _ = app.at("/*").serve_dir("static/telemetry/");

    let _ = app.at("/index.html").get(|_| async move {
        Ok(tide::Response::builder(200).body(Body::from_bytes(CONTENT_INDEX_HTML.into())).content_type(mime::HTML).build())
    });

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
                              //TODO: here is the defintion of the channels you need.


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

                              total_send.iter()
                                        .zip(total_take.iter())
                                        .enumerate()
                                        .for_each(|(i,(s,t))| dot_state.edges[i].compute_and_refresh(s,t));

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
                                   state == &GraphRuntimeState::StopInProgress
                                || state == &GraphRuntimeState::StopRequested
                                || state == &GraphRuntimeState::Stopped
                            };

                            history.update(dot_state.seq,flush_all).await;

                                  //must mark this for next time
                                  history.mark_position();
                            }


                         },

                        Err(msg) => {error!("Unexpected error on incomming message: {}",msg)}
                  }

            }
        }
    }

    Ok(())
}

struct InternalState {
    sequence: u128,
    actor_count: usize,
    total_sent: Vec<u128>,
    total_take: Vec<u128>,

    previous_total_take: Vec<u128>,
    ma_index: usize,
    ma_consumed_buckets:Vec<[u128; MA_ADJUSTED_BUCKETS]>,
    ma_consumed_runner: Vec<u128>,
    ma_inflight_buckets:Vec<[u128; MA_ADJUSTED_BUCKETS]>,
    ma_inflight_runner: Vec<u128>,

}


