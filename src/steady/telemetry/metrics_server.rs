use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use async_std::sync::Mutex;
use crate::steady::{config, SteadyRx};
use crate::steady::telemetry::metrics_collector::DiagramData;
use crate::steady::dot::*;
use futures::FutureExt; // Provides the .fuse() method
use futures::pin_mut; // Add pin_mut

use bytes::BytesMut;
use futures::select;
use log::*;
use crate::steady::monitor::SteadyMonitor;
use tide::{Body, Middleware, Next, Request, Response};
use tide::http::mime;


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


pub(crate) async fn run(monitor: SteadyMonitor, rx: Arc<Mutex<SteadyRx<DiagramData>>>) -> std::result::Result<(),()> {

    let dot_graph = BytesMut::new();

    let mut state = InternalState {
        sequence: 0,
        actor_count: 0,
        total_take: Vec::new(), //running totals
        total_sent: Vec::new(), //running totals


        previous_total_take: Vec::new(), //for measuring cycle consumed
        ma_index: 0,
        ma_consumed_buckets: Vec::new(),
        ma_consumed_runner: Vec::new(),
        ma_inflight_buckets: Vec::new(),
        ma_inflight_runner: Vec::new(),
    };

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
    let mut dot_graph = BytesMut::new();

// Define a new instance of the state.
    let mut state = Arc::new(Mutex::new(State {
        doc: Vec::new()
    }));

    let mut app = tide::with_state(state.clone());

    app.at("/").serve_file("static/telemetry/index.html");
    app.at("/*").serve_dir("static/telemetry/");

    app.at("/graph.dot")
        .get(|req: Request<Arc<Mutex<State>>>| async move {
        let vec = {
            let guard = req.state().lock().await;
            let state = guard.deref();
            state.doc.clone()
        };
        let mut res = Response::new(200);
        res.set_content_type(mime::PLAIN);  // Set the content type to text/plain
        res.set_body(Body::from_bytes(vec));
        Ok(res)
    });



    let server_handle = app.listen("127.0.0.1:8080");
    pin_mut!(server_handle);

    loop {
        select! {
            _ = server_handle.as_mut().fuse() => {
                println!("Web server exited.");
                break;
            },
            msg = rx.take_async().fuse() => {
                let seq = update_dot(&mut dot_graph, rankdir, &mut dot_state, msg);
                let vec = dot_graph.to_vec();

                let mut state = state.lock().await;
                let mystate = state.deref_mut();
                mystate.doc = vec;
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

/*
 ////////////////////////////////////////////////
        //always compute our moving average values
        ////////////////////////////////////////////////
        //NOTE: at this point we must compute rates so its done just once

        //This is how many are in the channel now
        let total_in_flight_this_cycle:Vec<u128> = state.total_sent.iter()
                                                        .zip(state.total_take.iter())
                                                        .map(|(s,t)| s.saturating_sub(*t)).collect();
        //This is how many messages we have consumed this cycle
        let total_consumed_this_cycle:Vec<u128> = state.total_take.iter()
                                                       .zip(state.previous_total_take.iter())
                                                       .map(|(t,p)| t.saturating_sub(*p)).collect();

        //message per second is based on those TAKEN/CONSUMED from channels
        state.ma_consumed_buckets.iter_mut()
                .enumerate()
                .for_each(|(idx,buf)| {
                    state.ma_consumed_runner[idx] += buf[state.ma_index];
                    state.ma_consumed_runner[idx] = state.ma_consumed_runner[idx].saturating_sub(buf[(state.ma_index + MA_ADJUSTED_BUCKETS - MA_MIN_BUCKETS ) & MA_BITMASK]);
                    buf[state.ma_index] = total_consumed_this_cycle[idx];
                    //we do not divide because we are getting the one second total
                });

        //ma for the inflight
        let ma_inflight_runner:Vec<u128> = state.ma_inflight_buckets.iter_mut()
            .enumerate()
            .map(|(idx,buf)| {
                state.ma_inflight_runner[idx] += buf[state.ma_index];
                state.ma_inflight_runner[idx] = state.ma_inflight_runner[idx].saturating_sub(buf[(state.ma_index + MA_ADJUSTED_BUCKETS - MA_MIN_BUCKETS ) & MA_BITMASK]);
                buf[state.ma_index] = total_in_flight_this_cycle[idx];
                //we do divide to find the average held in channel over the second
                state.ma_inflight_runner[idx]/MA_MIN_BUCKETS as u128
            }).collect();

        //we may drop a seq no if backed up
        //warn!("building sequence {} in metrics_collector with {} consumers",seq,fixed_consumers.len());

        let all_room = true;//confirm_all_consumers_have_room(&fixed_consumers).await;   //TODO: can we make a better loop?
        if all_room && !fixed_consumers.is_empty() {
            //info!("send diagram edge data for seq {}",seq);
            /////////////// NOTE: if this consumes too much CPU we may need to re-evaluate
            let total_take              = Arc::new(state.total_take.clone());
            let consumed_this_cycle     = Arc::new(total_consumed_this_cycle.clone());
            let ma_consumed_per_second  = Arc::new(state.ma_consumed_runner.clone());
            let in_flight_this_cycle    = Arc::new(total_in_flight_this_cycle);
            let ma_inflight_per_second  = Arc::new(ma_inflight_runner.clone());



       //keep current take for next time around
        state.previous_total_take.clear();
        state.total_take.iter().for_each(|f| state.previous_total_take.push(*f) );

        //next index
        state.ma_index = (1+state.ma_index) & MA_BITMASK;
 */
