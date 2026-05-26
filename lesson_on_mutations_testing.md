
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Lesson: Mutation Testing with cargo-mutants — Closing the Last Gaps in Steady State Verification                                                ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

You already have proptest for structured property checking and fuzzing for unknown-unknown crash hunting. 

This lesson adds the final layer of the modern Rust verification stack: **mutation testing** with `cargo-mutants`.

Mutation testing deliberately injects tiny bugs (“mutants”) into your code and checks whether your existing tests catch them. If a mutant survives (i.e. all tests still pass), you have a gap in your test suite.

**Zero Steady State-specific framework changes are required** — everything works out of the box with your existing proptests and full-graph tests. However, Steady State’s reactive actor style introduces one important practical detail you must handle: **timeouts caused by mutations to control-flow code**.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Why Mutation Testing Still Matters (Even After Proptest + Fuzz)

Proptest and fuzzing are powerful, but they can miss subtle logic errors:

- A mutant that breaks FizzBuzz logic but your proptest only checks “length is correct”.
- A mutant that removes a `mark_closed()` call or flips a backpressure condition.
- A mutant that corrupts the `is_running` veto logic.

`cargo-mutants` finds exactly these gaps by asking: “What if this line was wrong — would my tests notice?”

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Setup (One-Time, Trivial)

```bash
cargo install cargo-mutants
```

Add the helper crate to `[dev-dependencies]` (zero runtime cost):

```toml
[dev-dependencies]
mutants = "0.0.3"   # for #[mutants::skip] attribute
```

No other changes to `Cargo.toml` or your actors are needed.

Start narrow:

```bash
cargo mutants --file src/actor/          # focus only on your actors first
```

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### 1. Individual Actor Mutation Testing

Your proptests that call `internal_behavior` directly are excellent mutation targets. No changes required.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### 2. Full-Graph Mutation Testing

Mutating your production `build_graph`, channel wiring, or full-graph proptests works automatically because you reuse the exact same test code.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### 3. Steady State-Specific Detail: Handling Timeouts and Hang-Prone Code

**This is the only Steady State-specific consideration.**

Steady State actors spend most of their time inside `is_running` loops, `await_for_all!` / `await_for_any!` macros, veto closures, and backpressure waits. Mutations to these control-flow constructs often cause the test to **hang** instead of failing cleanly.

`cargo-mutants` handles this safely:

- It enforces a **per-mutant timeout** (default ≈ 5× your baseline test runtime, with a hard minimum of ~20 seconds).
- When a test times out, `cargo-mutants` kills the process and records the result as **TIMEOUT**.
- **TIMEOUT is treated as a caught/killed mutant** — it does *not* count as a survived mutant that you have to fix.

However, these timeouts create noise on every run. The clean, idiomatic solution is to **explicitly document and skip** the hang-prone sections using the `#[mutants::skip]` attribute.

#### Recommended Places to Add `#[mutants::skip]` in Steady State Code

```rust
// Typical is_running loop — very common source of timeouts
#[mutants::skip]  // mutations here cause hangs/timeouts; proptest + fuzz already cover shutdown/veto logic well
while actor.is_running(|| 
    i!(heartbeat_rx.is_closed_and_empty()) &&
    i!(generator_rx.is_closed_and_empty()) &&
    i!(logger_tx.mark_closed())
) { ... }
```

```rust
// On internal_behavior functions
#[mutants::skip]  // control-flow heavy; timeout-prone under mutation
async fn internal_behavior<A: SteadyActor>(mut actor: A, ...) -> Result<(), Box<dyn Error>> { ... }
```

```rust
// On full-graph test helpers or StageManager usage
#[mutants::skip]  // test orchestration can hang when mutated
fn run_full_graph_test(...) { ... }
```

You can also skip entire files or patterns in `.cargo/mutants.toml`:

```toml
# .cargo/mutants.toml
exclude_files = ["src/actor/common.rs"]
timeout = 15          # seconds — tune to your normal test duration
```

**Why this is the right approach for Steady State**:
- It removes noise without losing value.
- The attribute serves as living documentation: “We deliberately do not mutate this control flow because our higher-level tests already protect it.”
- It keeps mutation runs fast and focused on the parts that matter (business logic, state transformations, error paths).

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Best Practices & Gotchas

1. **Start narrow** — Always begin with `--file src/actor/` or a single actor file.
2. **Tune timeout** — Use `--timeout 15` (or set it in `.cargo/mutants.toml`) so hangs are killed quickly.
3. **Use `#[mutants::skip`** generously on `is_running`, veto closures, and `await_for_*!` macros.
4. **Interpret TIMEOUTs** — They are usually harmless noise, not a failure of your test suite.
5. **Combine with Tracey** — Add `r[verify ...]` comments to your proptests so you can see which requirements are mutation-covered.
6. **Run periodically** — Use mutation testing as a pre-release or nightly audit, not on every commit.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

### Summary Table: Where Mutation Testing Fits in Steady State

| Test Layer             | Primary Strength                     | Mutation Testing Value                          | Steady State Note                     |
|------------------------|--------------------------------------|-------------------------------------------------|---------------------------------------|
| Proptest               | Structured properties                | Finds weak properties                           | Excellent coverage                    |
| Fuzzing                | Crash discovery                      | Finds silent logic bugs                         | Excellent coverage                    |
| Full-graph tests       | Wiring + shutdown                    | Catches subtle veto/backpressure errors         | Timeout-prone control flow            |
| **cargo-mutants**      | —                                    | Proves your tests actually matter               | Use `#[mutants::skip]` on loops      |

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

**Final Lesson:**  
Steady State’s clean, deterministic test harness makes mutation testing extremely effective — you get the full power of `cargo-mutants` with zero framework changes. The only Steady State-specific practice is to add `#[mutants::skip]` on the control-flow heavy sections (`is_running`, veto closures, `await_for_all!`, etc.) so that timeout noise is eliminated and the tool stays focused on the logic that matters.

With this small, well-documented adjustment you now have the complete maximum truth-seeking verification stack:

- Spec traceability (Tracey)
- Property-based testing (proptest)
- Coverage-guided exploration (fuzzing)
- Mutation coverage (cargo-mutants)

Run it on your most critical actors first. Every survived mutant you kill (and every `#[mutants::skip]` you add) will make your actor system more robust.

Happy mutating!

┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
