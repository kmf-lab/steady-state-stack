
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Lesson: Fuzz Testing with cargo-fuzz — Hunting the Unknown Unknowns in Steady State                                                             ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

You have already read and applied the **lesson_on_proptest.md**. You are now getting massive value from structured, property-driven testing: thousands of 
generated cases, perfect shrinking, and rock-solid invariants on both individual actors and full graphs.

This lesson shows you the **next layer** of maximum truth-seeking: **coverage-guided fuzz testing** with `cargo-fuzz` + libFuzzer.

While proptest is hypothesis-driven ("for all inputs that match my strategy, these properties must hold"), fuzzing is exploration-driven. It mutates 
inputs relentlessly to maximize code coverage and surface crashes, panics, and unexpected paths you never thought to write a property for.

Steady State + proptest already gives you excellent testing. Adding fuzzing gives you **maximum** testing.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Why Fuzzing + Steady State (on top of Proptest) Is Extremely Powerful

Proptest excels at the cases you can articulate. Fuzzing excels at the cases you *cannot* articulate yet.

Because you already have proptest in place, the jump to fuzzing is **much smaller** than it would be for a normal project:

- Your proptest strategies become perfect `Arbitrary` implementations.
- Your test helpers (`build_test_graph`, injection logic, assertion macros) can be shared directly.
- The same `GraphBuilder::for_testing()` + `StageManager` + `internal_behavior` deterministic harness works unchanged.
- No new threading headaches — everything stays single-threaded and deterministic in test mode.

**Benefits you gain by layering fuzzing on top of proptest**:

| Benefit                              | Proptest Alone                     | Proptest + cargo-fuzz                          |
|--------------------------------------|------------------------------------|------------------------------------------------|
| Known invariant coverage             | Excellent                          | Excellent (unchanged)                          |
| Unknown edge-case discovery          | Good (limited by your strategies)  | **Outstanding** (coverage-guided mutation)     |
| Panic / crash hunting                | Only what your properties catch    | Finds hidden panics in every code path         |
| State-machine / shutdown robustness  | Strong                             | Exhaustive (exercises every branch)            |
| Shrinking / debuggability            | Best-in-class                      | Good (libFuzzer + corpus minimization)         |
| CI integration                       | Normal `cargo test`                | Dedicated long-running jobs or nightly         |
| Setup cost (now that you have proptest) | —                                | **Very low** (reuse everything)                |

This combination is one of the strongest verification stories available in Rust for concurrent actor systems.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Setup (One-Time, Leverages Your Existing Proptest Work)

1. Add to `Cargo.toml` (in the `[dev-dependencies]` or a new `[[bin]]` section):
   ```toml
   [dev-dependencies]
   cargo-fuzz = "0.12"      # or latest
   arbitrary = { version = "1", features = ["derive"] }
   ```

2. Create the fuzz directory:
   ```bash
   cargo fuzz init
   ```

3. Your existing proptest strategies become the foundation:
   - Add `#[derive(Arbitrary)]` to the same structs you use in proptest.
   - Or use `proptest-arbitrary-interop` to share strategies directly.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### 1. Individual Actor Fuzz Tests — Reusing Your Proptest Logic

Because you already call `internal_behavior` directly in proptests, turning one into a fuzz target is almost copy-paste.

#### Pattern (built directly on your proptest work)

```rust
// fuzz/fuzz_targets/actor_generator.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use steady_state::*;

// Reuse the exact same input type you already use in proptest
#[derive(Arbitrary, Debug)]
struct GeneratorFuzzInput {
    count: u64,
    // any other fields your proptest strategy already generates
}

fuzz_target!(|input: GeneratorFuzzInput| {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(1024).build::<u64>();

    let state = new_state::<GeneratorState>();

    graph.actor_builder()
        .with_name("Generator")
        .build(move |c| internal_behavior(c, tx.clone(), state.clone()), SoloAct);

    graph.start();
    graph.request_shutdown();
    let _ = graph.block_until_stopped(Duration::from_secs(2));

    let produced = rx.testing_take_all();
    // Reuse the same assertions you already wrote for proptest
    assert_eq!(produced.len() as u64, input.count);
    // or let it panic — the fuzzer will find it
});
```

**Benefits of this style**:
- Sub-millisecond per fuzz iteration.
- Perfect reuse of your proptest helpers and assertions.
- Finds panics or logic errors in `internal_behavior` that your proptest strategies missed.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### 2. Full-Graph Fuzz Tests — The Real Superpower

This is where fuzzing shines brightest. You fuzz the **exact same production graph** you already test with proptest.

#### Core Pattern (minimal change from your full-graph proptest)

```rust
// fuzz/fuzz_targets/full_graph.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use steady_state::*;

#[derive(Arbitrary, Debug)]
struct FullGraphFuzzInput {
    gen_messages: Vec<u64>,
    hb_beats: Vec<u64>,
    // add any other random parameters you already fuzz in proptest
}

fuzz_target!(|input: FullGraphFuzzInput| {
    let mut graph = GraphBuilder::for_testing().build(MainArg::default());
    build_graph(&mut graph);               // ← exact production wiring!

    graph.start();
    let mut stage = graph.stage_manager();

    // Reuse the exact same injection logic from your proptest
    // generator_tx.testing_send_all(input.gen_messages.clone(), false);
    // or use StageManager performs

    stage.final_bow();
    graph.request_shutdown();
    let _ = graph.block_until_stopped(Duration::from_secs(3));

    // Reuse your proptest-style system-wide assertions
    // or simply let any panic be discovered by the fuzzer
});
```

#### Advanced Full-Graph Techniques (building on proptest)

- **Random early shutdown + veto fuzzing** — let the fuzzer choose when to call `request_shutdown`.
- **State persistence across simulated restarts** — run the graph twice in one fuzz target and verify `SteadyState<T>` survives.
- **Backpressure & capacity torture** — generate huge vectors that stress `AwaitForRoom` and `is_running` logic.
- **Shared helpers** — extract `run_full_graph_test(input)` from your proptest file and call it from both proptest **and** fuzz targets.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### 3. Why This Layer Gives You Maximum Truth-Seeking

- Proptest finds bugs in the properties you wrote.
- Fuzzing finds bugs in the properties you *forgot* to write.
- Together they give near-exhaustive coverage of your actor logic, wiring, shutdown vetoes, backpressure, and state management.
- The actor-per-core threading model is completely invisible in fuzz mode (same as proptest).
- You get the mechanical sympathy of production **and** the deepest possible verification.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Best Practices & Gotchas

1. **Start small** — convert one existing proptest (individual actor) into a fuzz target first.
2. **Share code aggressively** — put common test helpers in `tests/common.rs` or a `fuzz_utils` module.
3. **Use `#[derive(Arbitrary)]` + proptest strategies** — the `arbitrary` crate works beautifully with the same types you already use.
4. **Run fuzzers locally first** — `cargo fuzz run full_graph -- -max_total_time=300` (5 minutes).
5. **Corpus management** — keep interesting inputs in `fuzz/artifacts/` and `fuzz/corpus/`.
6. **CI** — run fuzzers in nightly jobs or on dedicated machines (not every PR).
7. **Always drop StageManager** — same rule as proptest (`final_bow()` before shutdown).
8. **Nightly Rust** — some fuzz targets need it; use `cargo +nightly fuzz run ...`.

**Common pitfall**: Forgetting that fuzz targets must not panic on *expected* paths. Use `assert!` or let unexpected panics be found — both are valid.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Summary Table: Proptest vs. Fuzzing (Layered Approach)

| Test Goal                              | Proptest (Structured)             | Fuzzing (Coverage-Guided)         | Best Used Together? |
|----------------------------------------|-----------------------------------|-----------------------------------|---------------------|
| Verify explicit properties             | Primary                           | Secondary                         | Yes                 |
| Discover unknown edge cases            | Good                              | Excellent                         | Yes                 |
| Find hidden panics / crashes           | Limited                           | Excellent                         | Yes                 |
| Speed per iteration                    | Very fast                         | Fast (but run longer)             | Yes                 |
| Code reuse from existing tests         | —                                 | Extremely high                    | Yes                 |
| Shrinking / minimal failing case       | Best                              | Good                              | Yes                 |

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

**Final Lesson:**  
Proptest gave you confidence in the cases you could describe. Fuzz testing now gives you confidence in the cases you *couldn’t* describe. 

Because Steady State’s testing architecture (`GraphBuilder::for_testing()`, deterministic puppets, direct `internal_behavior` access) is already proptest-native, adding `cargo-fuzz` is almost free — and the payoff is enormous for any production-grade actor system.

You now have the complete modern Rust verification stack: unit tests → proptest properties → coverage-guided fuzzing. This is maximum truth-seeking.

Start with one fuzz target today. You will sleep even better knowing the fuzzer is hunting 24/7 for anything your proptests might have missed.

Happy fuzzing!

┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
