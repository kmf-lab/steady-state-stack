
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Lesson: Property-Based Testing with Proptest — Bulletproof Actors and Full Graphs in Steady State                                                ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

Steady State was designed from the ground up with **testability as a first-class architectural concern**. The `run`/`internal_behavior` dispatcher, 
`GraphBuilder::for_testing()`, the `StageManager`, and the deterministic puppet execution model were all built to make rigorous, automated testing not 
just possible — but *pleasant* and *scalable*.

This lesson teaches you how to combine Steady State with the **proptest** crate to achieve the highest possible confidence in your actor logic, 
wiring, shutdown behavior, backpressure handling, and state persistence — all with minimal boilerplate.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Why Proptest + Steady State Is a Perfect Match

Traditional unit tests and even integration tests check only the cases you remember to write. Property-based testing (PBT) with proptest does something 
radically more powerful:

- It **generates thousands of random inputs** (message sequences, batch sizes, timing signals, shutdown points, partial data, etc.).
- It **shrinks** failing cases automatically to the smallest possible input that triggers the bug.
- It runs your test **deterministically** every time.

Most actor frameworks (especially tokio-based ones) fight proptest because their test harnesses introduce non-determinism, global runtimes, thread pools, 
and complex async setup. Steady State does the opposite:

- `GraphBuilder::for_testing()` disables **all** real OS threading, CoreBalancer, SoloAct, and core affinity.
- Everything runs synchronously on the **single test thread** as deterministic puppets.
- You reuse your exact production `build_graph` and `internal_behavior` code.
- No tokio, no global executor, no hidden state.

Result: proptest can explore millions of scenarios safely, quickly, and repeatably.

**Benefits you will actually get**:

| Benefit                              | Traditional Tests                  | Proptest + Steady State                          |
|--------------------------------------|------------------------------------|--------------------------------------------------|
| Edge-case discovery                  | Manual, incomplete                 | Automatic, exhaustive                            |
| Minimal failing examples             | You hunt for them                  | Automatic shrinking                              |
| Shutdown / veto / backpressure       | Rarely tested                      | Naturally exercised by random inputs             |
| State persistence across restarts    | Hard to simulate                   | Easy with `SteadyState<T>` + `try_lock_sync`    |
| Production code reuse                | Often duplicated                   | 100% (same `build_graph` + `internal_behavior`) |
| Test speed                           | Fast                               | **Extremely** fast (no threads)                  |
| Confidence in full system            | Low                                | Extremely high                                   |

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### 1. Individual Actor Proptests — Testing the "Brain" (`internal_behavior`)

The golden rule from `lesson-on-actor-testing.md` still applies: **never call `run` in unit tests**. Call `internal_behavior` directly.

#### Why This Works So Well with Proptest
- `internal_behavior` is pure domain logic + deterministic channel guards.
- You control every input and can assert every output/state invariant.
- Proptest can generate complex sequences, partial batches, error conditions, etc.

#### Pattern (copy-paste ready)

```rust
use proptest::prelude::*;
use steady_state::*;

proptest! {
    #[test]
    fn proptest_generator_produces_sequence(
        count in 0..500u64
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(1024).build::<u64>();

        let state = new_state::<GeneratorState>();

        graph.actor_builder()
            .with_name("Generator")
            .build(move |c| internal_behavior(c, tx.clone(), state.clone()), SoloAct);

        graph.start();

        // Inject nothing — the actor itself produces data
        // (or use testing_send_all on other inputs if it has them)

        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(2)).unwrap();

        let produced: Vec<u64> = rx.testing_take_all();
        prop_assert_eq!(produced.len() as u64, count); // or whatever your invariant is
        // more assertions on ordering, state, etc.
    }
}
```

**Realistic example from the standard project** (Worker actor):

```rust
proptest! {
    #[test]
    fn proptest_worker_fizzbuzz_logic(
        input_values in prop::collection::vec(0u64..10000, 0..200),
        heartbeat_count in 1..10u64
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (gen_tx, gen_rx) = graph.channel_builder().build::<u64>();
        let (hb_tx, hb_rx) = graph.channel_builder().build::<u64>();
        let (log_tx, log_rx) = graph.channel_builder().build::<FizzBuzzMessage>();

        let state = new_state(); // if worker had state

        graph.actor_builder()
            .with_name("Worker")
            .build(move |c| internal_behavior(c, hb_rx.clone(), gen_rx.clone(), log_tx.clone()), SoloAct);

        graph.start();

        gen_tx.testing_send_all(input_values.clone(), true);
        hb_tx.testing_send_all(vec![0; heartbeat_count as usize], true);

        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(2)).unwrap();

        let output = log_rx.testing_take_all();
        prop_assert_eq!(output.len(), input_values.len());
        // You can even write a property that every output matches FizzBuzzMessage::new(...)
    }
}
```

**Benefits of this style**:
- Extremely fast (sub-millisecond per case).
- Perfect for deep mathematical / state-machine logic.
- Proptest shrinking gives you the smallest input vector that breaks your actor.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### 2. Full-Graph Proptests — Testing the "Wiring" + System Behavior

This is where proptest shines brightest. You test the **entire production graph** with zero duplication.

#### The Core Pattern (used in `main_tests::graph_test`)

```rust
proptest! {
    #[test]
    fn proptest_full_standard_graph(
        gen_messages in prop::collection::vec(0u64..1000, 0..300),
        hb_beats in prop::collection::vec(0u64..5, 1..15),
        shutdown_after_beats in 0..20usize,   // fuzz early shutdowns
    ) {
        let mut graph = GraphBuilder::for_testing().build(MainArg::default());
        build_graph(&mut graph);               // ← exact production wiring!

        graph.start();
        let mut stage = graph.stage_manager();

        // Option 1: Direct injection (fastest)
        // generator_tx.testing_send_all(gen_messages.clone(), false);
        // heartbeat_tx.testing_send_all(hb_beats.clone(), false);

        // Option 2: StageManager "god mode" (more powerful)
        stage.actor_perform(NAME_GENERATOR, StageDirection::Echo(gen_messages.clone()))?;
        // ... interleave other performs, force partial batches, etc.

        // Random early shutdown to test veto / liveliness
        if shutdown_after_beats < hb_beats.len() {
            // simulate mid-run shutdown
        }

        stage.final_bow();                     // MUST drop before shutdown
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(3)).unwrap();

        // System-wide invariants
        prop_assert!(/* all channels drained */);
        prop_assert!(/* final SteadyState values make sense */);
        // check telemetry, no panics, correct end-to-end transformation, etc.
    }
}
```

#### Advanced Full-Graph Techniques

1. **Random shutdown timing + veto testing**
   ```rust
   // Inside the proptest:
   for _ in 0..shutdown_after_beats {
       stage.actor_perform(NAME_HEARTBEAT, StageDirection::Echo(0))?;
   }
   ```

2. **Testing state persistence across simulated restarts**
   - After first run, inspect `state.try_lock_sync()`.
   - Build a second graph and verify state is restored.

3. **Backpressure & capacity fuzzing**
   - Generate huge vectors that would overflow naive channels.
   - Assert the system never deadlocks and respects `AwaitForRoom`.

4. **Stateful property-based testing** (using `proptest-state-machine`)
   - Model the expected system state as a state machine.
   - Proptest drives random sequences of actions (send, shutdown, restart).

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### 3. Why This Approach Gives You Superpowers

- **Zero non-determinism** — actor-per-core threading is completely disabled in test mode.
- **Maximum code reuse** — your production `build_graph` and every `internal_behavior` are tested exactly as they run in production.
- **Shrinking magic** — when a rare edge case fails, proptest gives you the exact minimal input that reproduces it.
- **Confidence at scale** — run 10,000+ cases in CI with `--test-threads=1` (still fast).
- **Catches the bugs that matter** — race conditions in shutdown, incorrect backpressure, broken invariants under weird message ordering, state corruption on restart.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Best Practices & Gotchas

1. Always start with `GraphBuilder::for_testing()`.
2. Always drop `StageManager` (`final_bow()` or `drop(stage)`) **before** `request_shutdown`.
3. Use `testing_send_all(..., true)` for final data injection in simple cases.
4. Keep test graphs small when running thousands of proptest cases.
5. Combine both styles:
   - Individual actor proptests → deep logic
   - Full-graph proptests → wiring + system invariants
6. Use the `i!` macro in `is_running` closures — it gives beautiful failure diagnostics.
7. Add custom `StageAction` impls for "god mode" internal state poking when needed.

**Common pitfall**: Forgetting to call `final_bow()` → test hangs. The framework will tell you exactly why via veto reasons.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Summary Table: When to Use Which Style

| Test Goal                              | Use Individual Actor (`internal_behavior`) | Use Full Graph (`StageManager` + `build_graph`) |
|----------------------------------------|--------------------------------------------|-------------------------------------------------|
| Pure logic / math / state machine      | Yes (primary)                              | Secondary                                       |
| Message transformation invariants      | Yes                                        | Yes                                             |
| Wiring / channel interactions          | No                                         | Yes (primary)                                   |
| Shutdown / veto / backpressure         | Partial                                    | Yes                                             |
| End-to-end system properties           | No                                         | Yes                                             |
| Speed (thousands of cases)             | Fastest                                    | Still very fast                                 |

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

**Final Lesson:**  
Steady State + proptest is not just "nice to have" — it is the most powerful way to verify concurrent actor systems in Rust today. You get the mechanical sympathy and low-latency threading of production **and** the exhaustive, shrinkable, deterministic testing that proptest was built for.

The framework has already done the hard work. All you have to do is write the properties.

Start small: add one proptest for your most complex actor’s `internal_behavior`, then graduate to a full-graph proptest that exercises random shutdowns. You will find bugs you never knew existed — and you will sleep much better at night.

Happy property hunting!

┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
