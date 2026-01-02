┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Lesson: Orchestrating Performance—Solo Actors vs. Actor Troupes                                                                                          ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

In Steady State, how you map your actors to OS threads is one of the most critical decisions for performance, latency, and resource utilization. This lesson
explores the two primary scheduling models: Solo Actors and Actor Troupes, along with advanced techniques for thread affinity and stack management.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

1. The Two Scheduling Models

Solo Actors (ScheduleAs::SoloAct)

A Solo Actor is assigned its own dedicated OS thread.

 • When to use: For heavy CPU-bound tasks, actors that perform blocking I/O (via call_blocking), or "driver" actors that must never be delayed by other
   logic.
 • Pros: Maximum isolation; no "noisy neighbor" effect from other actors.
 • Cons: High overhead if you have thousands of actors; increased context switching.

Actor Troupes (ScheduleAs::MemberOf)

A Troupe groups multiple actors onto a single OS thread. They execute cooperatively using an internal round-robin scheduler.

 • When to use: For high-density systems with thousands of actors, or when a group of actors works on the same data set and benefits from CPU cache
   locality.
 • Pros: Extremely low overhead; thousands of actors can run on a handful of threads.
 • Cons: If one actor in a troupe performs a long synchronous calculation, it delays every other actor in that troupe.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

2. Advanced Troupe Maneuvers: The "Linedance" Pattern

The Large Scale example demonstrates a sophisticated pattern where actors are moved between troupes during the graph-building phase. This allows you to
organize actors logically before committing them to hardware threads.

Dynamic Scheduling

You can decide at runtime whether an actor should be Solo or part of a Troupe using ScheduleAs::dynamic_schedule.


let mut my_troupe = Some(graph.actor_troupe());

// If the troupe exists, the actor joins it. If None, it becomes a SoloAct.
let schedule = ScheduleAs::dynamic_schedule(&mut my_troupe);

builder.build(move |c| my_actor::run(c), schedule);


The Transfer Trick

You can create multiple troupes and move actors between them using transfer_front_to or transfer_back_to. This is useful for "lining up" actors in a
specific execution order.


let mut troupe_a = graph.actor_troupe();
let mut troupe_b = graph.actor_troupe();

// ... build actors into troupe_a ...

// Move the last actor added to troupe_a over to troupe_b
troupe_a.transfer_back_to(&mut troupe_b);


Why do this? In complex graphs (like the Large Scale example), you might build a pipeline of actors (Generator -> Router -> User). By transferring them into
specific troupes, you ensure that the "User" actors share threads efficiently while the "Generators" stay isolated.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

3. CPU Affinity: Pinning Logic to Hardware

Steady State allows you to control exactly which CPU cores your actors and troupes inhabit. This is vital for reducing cache misses and avoiding "thread
migration" by the OS.

Explicit Core Assignment

Force an actor or troupe to a specific core.


base_actor_builder
    .with_explicit_core(3) // Pin to the 3rd CPU core (1-indexed for OS parity)
    .build(..., SoloAct);


Core Balancing and Exclusion

If you don't want to manage every core manually, use the CoreBalancer. You can also exclude specific cores (e.g., Core 1 and 2) to keep them clear for the
OS or high-priority drivers.


let balancer = graph.actor_builder().get_core_balancer();

base_actor_builder
    .with_core_balancing(balancer)
    .with_core_exclusion(vec![0, 1]) // Never use Core 0 or 1
    .build(..., SoloAct);


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

4. Managing the Stack

In high-scale systems, the default OS thread stack size (often 2MB or 8MB) can quickly exhaust your RAM. If you have 1,000 Solo Actors, 8MB each = 8GB of
RAM just for stacks!

Setting Stack Size

You can configure the stack size at the runner level or the individual actor level.


// Global default for all actors in the graph
SteadyRunner::release_build()
    .with_default_actor_stack_size(256 * 1024) // 256 KB per actor
    .run(args, |graph| { ... });

// Override for a specific heavy actor
graph.actor_builder()
    .with_stack_size(16 * 1024 * 1024) // 16 MB for this specific actor
    .build(..., SoloAct);


Lesson: Use small stacks (64KB - 256KB) for simple Troupes and large stacks only for actors performing deep recursion or large stack-allocated array
operations.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

5. The TroupeGuard Lifecycle

A TroupeGuard is a RAII (Resource Acquisition Is Initialization) object. The OS thread for the troupe is not spawned until the TroupeGuard is dropped.


{
    let mut troupe = graph.actor_troupe(); // Returns a TroupeGuard

    // Add 50 actors to the troupe
    for i in 0..50 {
        builder.build(..., ScheduleAs::MemberOf(&mut troupe));
    }

    info!("Actors added, but thread not yet started.");
} // <--- TroupeGuard is dropped here. The OS thread spawns NOW.


Lesson: This allows you to batch-initialize actors. If an actor in the troupe needs to reference another actor's channel, you can ensure all 50 are fully
configured before the first one starts executing.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

Summary Best Practices

 1 Default to Troupes: Start with actors in Troupes. Only move an actor to SoloAct if telemetry shows it is a bottleneck or if it performs blocking calls.
 2 Use i! in Troupes: Because troupe actors share a thread, a "zombie" actor that refuses to shut down will keep the entire troupe thread alive. Wrap your
   is_running conditions in i! to find the culprit.
 3 Match Girth to Cores: If you have a bundle with a Girth of 8, consider spawning 8 Solo Actors, each pinned to a unique core, for maximum parallel
   throughput.
 4 Monitor MCPU: Use the graph.dot telemetry. If a Troupe's MCPU is consistently near 1024, it means the OS thread is saturated. Split that troupe into two.

 ###################################
 
 ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Appendix: Creative Troupe Strategies and Core Management                                                                                                 ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

This appendix provides advanced patterns for mapping actors to hardware. In Steady State, a Core refers to a logical processor (thread) as seen by the OS,
and a Troupe is the mechanism used to multiplex actors onto those cores.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
1. Estimating Core Usage

Before deploying, use this formula to estimate your hardware requirements:

 • Solo Actors: 1 Core per Actor.
 • Troupes: 1 Core per Troupe (regardless of the number of actors inside).
 • System Actors: The framework uses ~2 Cores for telemetry and I/O drivers (Metrics Collector & Server).

Formula: Total Cores = (Count of SoloActs) + (Count of Troupes) + 2

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
2. Creative Usage: The "Single-Core Sandbox"

Objective: Ensure a group of non-critical actors (e.g., logging, cleanup, or background sync) never consumes more than 100% of a single CPU core, preventing
them from interfering with high-priority logic.


// All 100 background actors share exactly one OS thread.
let mut sandbox = graph.actor_troupe();
for i in 0..100 {
    builder.with_name_and_suffix("BackgroundWorker", i)
           .build(move |c| bg_task::run(c), ScheduleAs::MemberOf(&mut sandbox));
}
// Dropping the guard here pins all 100 to one core.


3. Creative Usage: The "Linedance" Pipeline

Objective: Use transfer_front_to to ensure that actors are processed in a specific order within the troupe's internal round-robin cycle.


let mut pipeline = graph.actor_troupe();

// We want the Decryptor to always run immediately after the Receiver
builder.build(move |c| receiver::run(c), ScheduleAs::MemberOf(&mut pipeline));
builder.build(move |c| decryptor::run(c), ScheduleAs::MemberOf(&mut pipeline));

// If we accidentally added a Logger in between, we can move it to the back
pipeline.transfer_front_to(&mut pipeline);


4. Creative Usage: NUMA-Aware Socket Pinning

Objective: On multi-socket servers, you want to keep a bundle and its actors on the same physical CPU to avoid the "NUMA penalty" (slow cross-socket memory
access).


// Assuming Cores 0-15 are on Socket A, and 16-31 are on Socket B
let socket_b_cores: Vec<usize> = (16..32).collect();

let mut socket_a_troupe = graph.actor_troupe();

builder.with_core_exclusion(socket_b_cores) // Force allocation to Socket A
       .build(move |c| high_speed_logic::run(c), ScheduleAs::MemberOf(&mut socket_a_troupe));


5. Creative Usage: The "I/O Shield"

Objective: Isolate actors that perform heavy call_blocking operations into their own troupe so they don't stall the main execution flow.


let mut io_troupe = graph.actor_troupe();

builder.with_name("DatabaseWriter")
       .with_stack_size(4 * 1024 * 1024) // Give I/O actors more stack for deep driver calls
       .build(move |c| {
           // This actor uses call_blocking, which spawns a temporary thread.
           // By being in a troupe, it doesn't block other SoloActors.
           db_actor::run(c)
       }, ScheduleAs::MemberOf(&mut io_troupe));


6. Creative Usage: The "Priority Lane"

Objective: Give a specific actor more "CPU time" by making it a SoloAct while keeping its peers in a Troupe.


// 10 normal workers share one core
let mut normal_lane = graph.actor_troupe();
for i in 0..10 {
    builder.build(..., ScheduleAs::MemberOf(&mut normal_lane));
}

// The VIP worker gets its own core entirely
builder.with_name("VIP_Worker")
       .with_explicit_core(1) // Pin to the first core for lowest latency
       .build(..., ScheduleAs::SoloAct);


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
7. Troubleshooting Core Contention

If your graph.dot telemetry shows an actor is "Stalled" or has high latency, check these three things:

 1 Troupe Saturation: Is the Troupe's MCPU near 1024? If so, move some actors to a new Troupe.
 2 Core Overlap: Did you use with_explicit_core on two different SoloActors for the same core? The OS will fight itself.
 3 Stack Overflow: Is an actor crashing and restarting? Check if your with_stack_size is too small for the data you are processing.

Pro Tip: Use i! in every is_running condition. In a Troupe, if one actor fails to check its shutdown signal, the entire OS thread stays alive, wasting a
core. The i! macro will tell you exactly which actor is the "anchor" preventing shutdown.

 
 