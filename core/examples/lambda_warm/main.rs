//! Warm Lambda-style graph: build once, stay Running, each inject is one in / one out.
//!
//! Honesty:
//! - Extra pack slots help **width**, not a sequential pipeline.
//! - Idle must be event-driven (`ingress.wait` / channel waits) — clocks lie across freeze.
//! - Persist before park when durable; `/tmp` does not outlive a new sandbox.
//! - No telemetry port in `for_lambda` (metrics features off).
//! - Cargo-lambda is a packager only; this example uses Steady’s blocking Runtime API.
//!
//! Run (local mock host loop — no AWS):
//! ```bash
//! cargo run -p steady_state --example lambda_warm --features lambda
//! ```
//!
//! With a real Runtime API (`AWS_LAMBDA_RUNTIME_API` set by the Lambda sandbox):
//! ```bash
//! cargo run -p steady_state --example lambda_warm --features lambda -- --runtime
//! ```

use clap::Parser;
use log::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use steady_state::*;

#[cfg(feature = "lambda")]
use steady_state::lambda_runtime_api::LambdaRuntimeApi;

#[derive(Parser, Debug, Clone)]
struct Args {
    /// Use AWS_LAMBDA_RUNTIME_API instead of the local demo inject loop.
    #[arg(long, default_value_t = false)]
    runtime: bool,
    /// Demo injects when not using --runtime.
    #[arg(long, default_value = "3")]
    injects: u64,
    #[arg(short = 'l', long = "loglevel", default_value = "info")]
    loglevel: LogLevel,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let _ = init_logging(args.loglevel, None);

    // for_lambda: telemetry off, strict persist, small stacks, pack detect (pinned here).
    let mut graph = GraphBuilder::for_lambda()
        .with_pack_slots(2)
        .build(args.clone());

    let (host, ingress, egress) = graph.lambda_bridge::<Vec<u8>, Vec<u8>>();
    let hits = Arc::new(AtomicU64::new(0));

    // Optional SoloAct VIP — reserves a pack slot; packed workers share the rest.
    graph.actor_builder().with_name("VIP_SOLO").build(
        move |ctx| async move {
            let mut actor = ctx.into_spotlight([], []);
            info!("VIP_SOLO on {:?}", thread::current().id());
            while actor.is_running(|| true) {
                // Long park interrupted by shutdown — not wait_periodic (freeze-safe).
                actor.wait(Duration::from_secs(3600)).await;
            }
            Ok(())
        },
        ScheduleAs::SoloAct,
    );

    let mut dyn_bag = graph.dynamic_troupe().with_name("LambdaPack");
    let worker_hits = hits.clone();
    graph.actor_builder().with_name("ECHO").build(
        move |ctx| {
            let ingress = ingress.clone();
            let egress = egress.clone();
            let worker_hits = worker_hits.clone();
            async move {
                let mut actor = ctx.into_spotlight([], []);
                info!("ECHO on {:?}", thread::current().id());
                while actor.is_running(|| true) {
                    let Some(body) = ingress.wait().await else {
                        break;
                    };
                    worker_hits.fetch_add(1, Ordering::SeqCst);
                    // Echo body; real apps parse JSON and call business logic.
                    egress.send(body);
                }
                Ok(())
            }
        },
        ScheduleAs::MemberOf(&mut dyn_bag),
    );
    drop(dyn_bag);

    graph.start();

    if args.runtime {
        #[cfg(feature = "lambda")]
        {
            // Host thread owns Runtime API; actors stay on SOLO/TROUP.
            let api = LambdaRuntimeApi::from_env()?;
            loop {
                match api.next_invocation() {
                    Ok(inv) => {
                        host.inject(inv.body);
                        match host.wait_output(Duration::from_secs(30)) {
                            Some(out) => {
                                if let Err(e) = api.post_response(&inv.request_id, &out) {
                                    error!("post_response: {e}");
                                }
                            }
                            None => {
                                let err = br#"{"errorMessage":"timeout","errorType":"Timeout"}"#;
                                let _ = api.post_error(&inv.request_id, err);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("next_invocation: {e}");
                        break;
                    }
                }
            }
            host.close();
            graph.request_shutdown();
        }
    } else {
        for i in 0..args.injects {
            let payload = format!(r#"{{"n":{i}}}"#).into_bytes();
            host.inject(payload.clone());
            let out = host
                .wait_output(Duration::from_secs(2))
                .expect("demo output");
            info!("inject {i} -> {}", String::from_utf8_lossy(&out));
        }
        host.close();
        graph.request_shutdown();
    }

    graph.block_until_stopped(Duration::from_secs(3))?;
    info!("done; echo hits={}", hits.load(Ordering::SeqCst));
    Ok(())
}
