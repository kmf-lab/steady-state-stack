┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Lesson: The Durable Core—Separating Logic from State with SteadyState<S>                                                                                 ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

In high-performance concurrent systems, failure isn't just a possibility—it’s a statistical certainty. Most actor frameworks treat an actor as a single unit
of logic and data. If that actor panics, the data dies with it.

Steady State changes this paradigm by introducing SteadyState<S>. This lesson explores why this separation is the secret to building "immortal" systems and
how it integrates with the framework's broader reliability features.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

1. The Philosophy: Ephemeral Actors, Durable State

In Steady State, Actors are ephemeral. They are designed to be stopped, started, moved, and regenerated. If an actor encounters a bug, the supervisor
restarts it.

State is durable. The SteadyState<S> container lives in the Graph, not the actor. When an actor is "regenerated" (restarted), the framework hands the new
instance the exact same state container used by its predecessor.

Why you need this:

Without this separation, a single null pointer or out-of-bounds error in a complex calculation would wipe out hours of accumulated telemetry, session data,
or progress. By using SteadyState<S>, you ensure that your system's "memory" survives even when its "brain" (the logic) needs a reboot.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

2. How it Works: The SteadyState Lifecycle

The SteadyState<S> container is an Arc<Mutex<Option<S>>>. It follows a strict lifecycle that guarantees thread safety and persistence.

Step 1: Definition (The Blueprint)

You define your state as a standard Rust struct. If you want disk persistence, you simply derive Serialize and Deserialize.


#[derive(Default, Serialize, Deserialize, Debug)]
pub struct ProcessingStats {
    pub total_processed: u64,
    pub failures: u32,
    pub last_checkpoint: Instant,
}


Step 2: Initialization (The Graph Root)

You create the state in main.rs. This is critical: because it is created in the graph root, it exists independently of the actor's specific OS thread.


// Option A: In-memory only (fast, survives actor restarts)
let state = new_state::<ProcessingStats>();

// Option B: Disk-backed (survives actor restarts AND full system reboots)
let state = new_persistent_state::<ProcessingStats, _>("./data/stats.json");


Step 3: Consumption (The Actor)

Inside the actor, you lock the state. The first time this is called, the provided closure initializes the data. On every subsequent restart (regeneration),
the closure is skipped, and the existing data is returned.


let mut guard = state.lock(|| ProcessingStats::default()).await;


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

3. Why This is Special: The "Zero-Effort" Persistence

The most powerful feature of SteadyState<S> is its integration with the filesystem. When you use new_persistent_state, Steady State provides a
"save-on-drop" guarantee.

 1 Automatic Loading: On startup, the framework checks the path. If stats.json exists, it populates your struct automatically.
 2 High-Speed Access: While the actor is running, you are interacting with raw memory. There is no disk I/O overhead during your work loop.
 3 Atomic-Style Saving: When the StateGuard is dropped (which happens if the actor finishes, panics, or the system shuts down), the framework automatically
   serializes the struct and writes it to disk.

This is a massive safety net. If your actor panics at 3:00 AM, the supervisor restarts it at 3:00:01 AM, and the actor finds its state exactly as it was one
millisecond before the crash.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

4. Integration with Reliability Features

SteadyState<S> does not work in a vacuum. It is part of a "Reliability Triad" provided by the framework:

I. The Supervisor (Regeneration)

The SteadyActorShadow tracks a regeneration count. You can use this inside your state to detect how many times your logic has failed and perhaps change
behavior (e.g., "If I've restarted 5 times, log a critical alert").

II. The is_running Loop & i! Macro

The is_running loop ensures that an actor only processes data when the graph is healthy. By wrapping your state checks in the i! macro, you provide the
telemetry system with "Veto Reasons." If an actor refuses to shut down, the telemetry will show: "Vetoed: state.has_pending_writes() is true."

III. Clean vs. Dirty Shutdowns

If a system is shut down via Ctrl-C, the is_running loop returns false. The actor then exits its loop, the StateGuard is dropped, and the final state is
safely flushed to disk. This turns a potentially "dirty" crash into a "clean" state save.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

5. Observability: The try_lock_sync Trick

One of the most confusing aspects of actor systems is that they are "black boxes." It’s hard to know what happened inside after they stop.

Steady State provides try_lock_sync() specifically for this. In your main.rs or in a unit test, after the graph has stopped, you can peek into the state:


graph.block_until_stopped(Duration::from_secs(5))?;

if let Some(final_stats) = state.try_lock_sync() {
    println!("Final count: {}", final_stats.total_processed);
}


This makes unit testing incredibly simple. You no longer need to mock complex output channels; you just check if the SteadyState contains the expected
values.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

6. Best Practices & Lessons

 • Don't use local mut variables for progress: If you find yourself writing let mut x = 0; outside your while actor.is_running loop, ask yourself: "Will I
   be sad if this resets to 0 after a crash?" If the answer is yes, move it into a SteadyState struct.
 • Keep it Lean: SteadyState is serialized as a single blob. Don't put a 500MB hashmap in there. Use it for metadata, counters, offsets, and small
   configuration states.
 • The "Panic Test": A great way to verify your reliability is to write a test that calls panic!() inside an actor. If you've used SteadyState correctly,
   the test should still pass because the actor will restart and complete its work.
 • Serde is Required: For persistent state, your struct must implement serde::Serialize and serde::DeserializeOwned.

By separating how you process (the Actor) from what you have processed (the State), you build a system that is not just fast, but truly resilient.

##############################

┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Appendix: Advanced Patterns and Technical Deep Dive for SteadyState<S>                                                                                   ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

This appendix provides technical details and practical code examples for implementing resilient state management in Steady State.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

1. Technical Mechanics: How Persistence Works

The SteadyState<S> container is more than just a Mutex. When you use new_persistent_state, the framework attaches an on_drop hook to the StateGuard.

 • The Guard: When an actor calls state.lock().await, it receives a StateGuard. This guard implements Deref and DerefMut, allowing you to treat it like your
   original struct.
 • The Trigger: The moment the StateGuard goes out of scope (is dropped), the on_drop closure is executed.
 • The Safety: Because the drop happens during a panic unwind, the framework captures the "last known good" state of your variables and writes them to disk
   before the actor thread actually terminates.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

2. Example: The Stream Offset Tracker

Scenario: You are consuming a stream (e.g., Aeron or a database log). You must ensure that if the actor crashes, it doesn't re-process the same messages
from the beginning.


#[derive(Default, Serialize, Deserialize)]
struct StreamState {
    last_processed_offset: u64,
}

async fn internal_behavior(
    mut actor: C,
    rx: SteadyRx<Message>,
    state: SteadyState<StreamState>
) -> Result<(), Box<dyn Error>> {
    // Load the last offset from disk/memory
    let mut guard = state.lock(|| StreamState::default()).await;

    // Logic: Skip already processed messages on restart
    let start_offset = guard.last_processed_offset;
    info!("Restarting from offset: {}", start_offset);

    while actor.is_running(&mut || rx.is_closed_and_empty()) {
        if let Some(msg) = actor.take_async(&mut rx).await {
            process_message(&msg);
            // Update the durable offset
            guard.last_processed_offset = msg.offset;
        }
    }
    Ok(())
}


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

3. Example: The "Circuit Breaker" (Using Regeneration)

Scenario: An actor keeps panicking because of a specific data-driven bug. You want to stop the actor permanently if it restarts more than 5 times to prevent
log spam or resource exhaustion.


async fn internal_behavior(mut actor: C, state: SteadyState<MyState>) -> Result<(), Box<dyn Error>> {
    // Check the framework-provided regeneration count
    if actor.regeneration() > 5 {
        error!("Critical failure: Actor regenerated too many times. Halting.");
        actor.request_shutdown().await;
        return Ok(());
    }

    let mut guard = state.lock(|| MyState::default()).await;
    // ... normal logic ...
}


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

4. Example: The Batch Accumulator

Scenario: You are batching 1,000 small messages into one large database write. If you panic at message 999, you don't want to lose the previous 998
messages.


#[derive(Default, Serialize, Deserialize)]
struct BatchState {
    pending_items: Vec<Message>,
}

async fn internal_behavior(...) {
    let mut guard = state.lock(|| BatchState::default()).await;

    while actor.is_running(...) {
        if let Some(msg) = actor.take_async(&mut rx).await {
            guard.pending_items.push(msg);

            if guard.pending_items.len() >= 1000 {
                db::write_batch(&guard.pending_items).await?;
                // Clear the durable state only after a successful write
                guard.pending_items.clear();
            }
        }
    }
}


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

5. Example: Post-Mortem Unit Testing

Scenario: You want to verify that an actor correctly calculated a complex sum, but you don't want to set up a mock receiver to catch the output.


#[test]
fn test_calculation_logic() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_testing().build(());
    let state = new_state::<MyCalcState>();
    let test_state = state.clone(); // Clone for the test thread

    // ... build and start graph ...
    graph.block_until_stopped(Duration::from_secs(1))?;

    // Use try_lock_sync to inspect the state after the actor is dead
    let final_data = test_state.try_lock_sync().expect("State should exist");
    assert_eq!(final_data.total_sum, 5000);
    Ok(())
}


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

6. Example: The Shutdown Veto

Scenario: An actor is in the middle of a sensitive multi-step operation. It should not shut down until the current step is finished.


while actor.is_running(&mut || {
    // We only allow shutdown if our state says we are 'idle'
    // Wrap in i! so telemetry shows WHY we are vetoing shutdown
    i!(guard.is_idle) && i!(rx.is_closed_and_empty())
}) {
    guard.is_idle = false;
    perform_sensitive_work().await;
    guard.is_idle = true;
}


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

7. Anti-Patterns: What to Avoid


  Anti-Pattern       Why it's bad                                                          The Correct Way
 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Large Blobs        Serializing a 1GB Vec on every panic/shutdown causes massive I/O      Use SteadyState for metadata/pointers; use a DB for large data.
                     lag.
  Local Mutables     let mut x = 0; outside the loop is lost on panic.                     Put x inside the SteadyState struct.
  Frequent Locking   Calling .lock().await inside a tight loop (e.g., 1M times/sec).       Call .lock().await once before entering the while loop.
  Non-Send Types     Putting a non-Send type (like a raw pointer) in state.                Ensure all state members are Send + Sync and Serialize.


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

8. Summary Checklist for Developers

 1 Does it need to survive a crash? Put it in SteadyState.
 2 Does it need to survive a reboot? Use new_persistent_state.
 3 Is it small? Keep the struct under a few megabytes.
 4 Is it initialized? Provide a sensible default in the .lock(|| ...) closure.
 5 Is it observable? Use try_lock_sync in your tests to validate the state.