//! Dynamic troupe packing demo.
//!
//! Maps a fixed actor graph onto this process’s CPU **slot budget** once at
//! `Graph::start` — not work-stealing, not live re-pack.
//!
//! Honesty:
//! - Extra cores help the **worker bundle** (parallel width), not a 2-node handshake.
//! - Kubernetes replica count is still HPA; this is **one process, this machine**.
//! - Pin slots with `SS_PACK_SLOTS` or `--slots` so demos are reproducible in cgroups.
//!
//! Run:
//! ```bash
//! cargo run -p steady_state --example dynamic_troupe -- --slots 2 --beats 8
//! SS_PACK_SLOTS=3 cargo run -p steady_state --example dynamic_troupe
//! ```

use clap::Parser;
use log::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use steady_state::*;

#[derive(Parser, Debug, Clone)]
struct Args {
    /// Detected CPU budget pin (`with_pack_slots`). Already-spawned SoloAct threads
    /// (VIP + telemetry) are subtracted. Default 5 → ~2 packed OS threads with VIP+telemetry.
    #[arg(long, default_value = "5")]
    slots: usize,
    /// Heartbeat iterations before shutdown.
    #[arg(long, default_value = "12")]
    beats: u64,
    #[arg(short = 'l', long = "loglevel", default_value = "info")]
    loglevel: LogLevel,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let _ = init_logging(args.loglevel, None);

    // Ops override for demos in constrained pods.
    // SAFETY: process-local demo only; not concurrent with other env readers here.
    unsafe {
        std::env::set_var("SS_PACK_SLOTS", args.slots.to_string());
    }

    info!(
        "dynamic_troupe demo: requested pack slots={} beats={} \
         (SoloAct VIP + telemetry each reserve threads from the budget)",
        args.slots, args.beats
    );

    // Keep telemetry so users can open http://127.0.0.1:9900; budget accounts for it.
    let mut graph = GraphBuilder::for_production()
        .with_pack_slots(args.slots)
        .with_telemtry_production_rate_ms(200)
        .build(args.clone());

    build_graph(&mut graph, args.beats);
    graph.start();
    graph.block_until_stopped(Duration::from_secs(30))
}

fn build_graph(graph: &mut Graph, beats: u64) {
    // Ingress → small-cap pipeline pair → fat-cap edge → worker bundle.
    let (ing_tx, ing_rx) = graph.channel_builder().with_capacity(2).build::<u64>();
    let (pipe_tx, pipe_rx) = graph.channel_builder().with_capacity(2).build::<u64>();
    let (fat_tx, fat_rx) = graph.channel_builder().with_capacity(4096).build::<u64>();
    let (work_tx, work_rx) = graph
        .channel_builder()
        .with_capacity(32)
        .build_channel_bundle::<u64, 2>();

    let mut dyn_bag = graph.dynamic_troupe().with_name("AppPack");

    // VIP SoloAct — occupies a reserved thread the packer does not reuse.
    let vip_hits = Arc::new(AtomicUsize::new(0));
    let vip = vip_hits.clone();
    graph.actor_builder().with_name("VIP_SOLO").with_thread_info().build(
        move |ctx| {
            let vip = vip.clone();
            async move {
                let mut actor = ctx.into_spotlight([], []);
                info!(
                    "VIP_SOLO on thread {:?} (SoloAct opt-out)",
                    thread::current().id()
                );
                while actor.is_running(|| true) {
                    actor.wait_periodic(Duration::from_millis(50)).await;
                    vip.fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            }
        },
        ScheduleAs::SoloAct,
    );

    // Heartbeat / ingress — packed.
    graph
        .actor_builder()
        .with_name("INGRESS")
        .with_thread_info()
        .with_pack_ports(vec![], vec![ing_tx.pack_channel_id()])
        .build(
            move |ctx| {
                let tx = ing_tx.clone();
                async move {
                    let mut actor = ctx.into_spotlight([], [&tx]);
                    let mut tx = tx.lock().await;
                    let mut n = 0u64;
                    info!("INGRESS on {:?}", thread::current().id());
                    while actor.is_running(|| tx.mark_closed()) {
                        await_for_all!(
                            actor.wait_periodic(Duration::from_millis(20)),
                            actor.wait_vacant(&mut tx, 1)
                        );
                        if actor.try_send(&mut tx, n).is_sent() {
                            n += 1;
                            if n >= beats {
                                actor.request_shutdown().await;
                            }
                        }
                    }
                    Ok(())
                }
            },
            ScheduleAs::MemberOf(&mut dyn_bag),
        );

    // Small-cap relay (should co-locate with ingress when slots are scarce).
    graph
        .actor_builder()
        .with_name("RELAY_SMALL")
        .with_thread_info()
        .with_pack_ports_from(&[&ing_rx], &[&pipe_tx])
        .build(
            move |ctx| {
                let rx = ing_rx.clone();
                let tx = pipe_tx.clone();
                async move {
                    let mut actor = ctx.into_spotlight([&rx], [&tx]);
                    let mut rx = rx.lock().await;
                    let mut tx = tx.lock().await;
                    info!("RELAY_SMALL on {:?}", thread::current().id());
                    while actor.is_running(|| {
                        i!(rx.is_closed_and_empty()) && i!(tx.mark_closed())
                    }) {
                        let clean = await_for_all!(
                            actor.wait_avail(&mut rx, 1),
                            actor.wait_vacant(&mut tx, 1)
                        );
                        if clean {
                            if let Some(v) = actor.try_take(&mut rx) {
                                let _ = actor.try_send(&mut tx, v);
                            }
                        }
                    }
                    Ok(())
                }
            },
            ScheduleAs::MemberOf(&mut dyn_bag),
        );

    // Fat-cap hop — often a separate cluster when slots ≥ 2.
    graph
        .actor_builder()
        .with_name("RELAY_FAT")
        .with_thread_info()
        .with_pack_ports_from(&[&pipe_rx], &[&fat_tx])
        .build(
            move |ctx| {
                let rx = pipe_rx.clone();
                let tx = fat_tx.clone();
                async move {
                    let mut actor = ctx.into_spotlight([&rx], [&tx]);
                    let mut rx = rx.lock().await;
                    let mut tx = tx.lock().await;
                    info!("RELAY_FAT on {:?}", thread::current().id());
                    while actor.is_running(|| {
                        i!(rx.is_closed_and_empty()) && i!(tx.mark_closed())
                    }) {
                        let clean = await_for_all!(
                            actor.wait_avail(&mut rx, 1),
                            actor.wait_vacant(&mut tx, 1)
                        );
                        if clean {
                            if let Some(v) = actor.try_take(&mut rx) {
                                let _ = actor.try_send(&mut tx, v);
                            }
                        }
                    }
                    Ok(())
                }
            },
            ScheduleAs::MemberOf(&mut dyn_bag),
        );

    // Fan-out to bundle workers (extra slots help here).
    graph
        .actor_builder()
        .with_name("FANOUT")
        .with_thread_info()
        .with_pack_ports(
            vec![fat_rx.pack_channel_id()],
            work_tx.iter().map(|t| t.pack_channel_id()).collect(),
        )
        .build(
            move |ctx| {
                let rx = fat_rx.clone();
                let txs = work_tx.clone();
                async move {
                    let mut actor = ctx.into_spotlight([&rx], [&txs[0], &txs[1]]);
                    let mut rx = rx.lock().await;
                    let mut t0 = txs[0].lock().await;
                    let mut t1 = txs[1].lock().await;
                    info!("FANOUT on {:?}", thread::current().id());
                    let mut lane = 0usize;
                    while actor.is_running(|| {
                        i!(rx.is_closed_and_empty())
                            && i!(t0.mark_closed())
                            && i!(t1.mark_closed())
                    }) {
                        let clean = await_for_all!(
                            actor.wait_avail(&mut rx, 1),
                            actor.wait_vacant(&mut t0, 1),
                            actor.wait_vacant(&mut t1, 1)
                        );
                        if clean {
                            if let Some(v) = actor.try_take(&mut rx) {
                                if lane % 2 == 0 {
                                    let _ = actor.try_send(&mut t0, v);
                                } else {
                                    let _ = actor.try_send(&mut t1, v);
                                }
                                lane += 1;
                            }
                        }
                    }
                    Ok(())
                }
            },
            ScheduleAs::MemberOf(&mut dyn_bag),
        );

    for (i, lazy_rx) in work_rx.iter().enumerate() {
        let pack_id = lazy_rx.pack_channel_id();
        let lazy_rx = lazy_rx.share();
        graph
            .actor_builder()
            .with_name_and_suffix("WORKER", i)
            .with_thread_info()
            .with_pack_ports(vec![pack_id], vec![])
            .build(
                move |ctx| {
                    let rx = lazy_rx.clone();
                    async move {
                        let mut actor = ctx.into_spotlight([&rx], []);
                        let mut rx = rx.lock().await;
                        info!("WORKER on {:?}", thread::current().id());
                        while actor.is_running(|| rx.is_closed_and_empty()) {
                            await_for_all!(actor.wait_avail(&mut rx, 1));
                            if let Some(v) = actor.try_take(&mut rx) {
                                trace!("worker got {v}");
                            }
                        }
                        Ok(())
                    }
                },
                ScheduleAs::MemberOf(&mut dyn_bag),
            );
    }

    // Dropping the dynamic bag before start is OK — Graph already owns the shared bag.
    drop(dyn_bag);
}
