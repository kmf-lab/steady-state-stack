# Steady State: LLM nomenclature and API clarity proposal

> **Status:** Phase 1 in execution for 0.3.0 (see §10). No crate renames in this workstream.  
> **Audience:** Steady State maintainers and agents preparing a future breaking/aliased release.  
> **Companion:** [`deep-actor-audit.md`](deep-actor-audit.md) audits actors against the **current** API.  
> **Wild code:** mvp-ingest, steady-llm, rhetor8-notary, and all lesson trees stay on current names until a deliberate migration.

---

## 1. Problem statement

Large language models are trained overwhelmingly on **Tokio-style push async**, **mutex lock/unlock**, **`recv().await` loops**, and **“don’t hold locks across `.await`”**. Steady State is a **pull-reactor** with **lifetime channel/state guards**, **peek-before-commit**, and **negotiated shutdown**. Correct Steady code therefore **looks wrong** to an LLM’s prior—and wrong Steady code **looks familiar**.

Empirically, agents fail Steady more often than they succeed when given only manifesto prose. The highest-leverage fix is **vocabulary and API shape**: names that either (a) match concepts the model already knows when the semantics match, or (b) **deliberately break** false friends when Steady semantics diverge.

### 1.1 Failure modes ranked by damage

| Rank | False friend | Typical LLM mistake | Production symptom |
|------|----------------|---------------------|--------------------|
| 1 | `.lock()` / “lock-first” | Unlock mid-await; lock inside loop; treat as cross-actor mutex | Deadlock “fixes” that unlock; panic loss; Phase-2 confused with unlock |
| 2 | Unit test via `run()` | Call `run` under `GraphBuilder::for_testing()` | Hang (puppet / simulation) |
| 3 | Scattered awaits | Nested `wait_*`, `recv().await` style | Spin, unfairness, unclean shutdown |
| 4 | `wait_periodic` as heartbeat | Always-on 50–250 ms poll for “liveliness” | CPU burn; mvp-ingest strict CI fail |
| 5 | Durable vs performant mix | `try_take` before work on durable path; peek on hot path without discipline | Lost messages on panic; or needless latency |
| 6 | Lazy vs Established | Pass `LazySteady*` into hot loop | Panic / affinity bugs |
| 7 | Multiplex OR vacant+avail | `await_for_any!(wait_vacant, wait_avail)` when vacant always ready | Starvation behind full TX |
| 8 | Shadow / Spotlight | Skip `into_spotlight`; wrong sim branch | Missing telemetry; wrong test mode |
| 9 | Veto / `mark_closed` | Treat shutdown as forced; forget producer close | Hang on `block_until_stopped` |
| 10 | Lane occupancy vs guards | “Hold lane until rings empty” | Fleet serialization (ingest-specific) |

---

## 2. Naming principles

1. **Never reuse “lock” for Steady channel/state binding.** Models treat “lock” as mutex with short critical sections. Prefer **guard**, **bind**, **acquire**.
2. **Prefer verbs that encode intent.** `wait_avail` is good; `recv` would be disastrous.
3. **One concept, one word.** “Established” appears in docs but not in type names—fix that mismatch.
4. **Mode names beat comments.** Durable peek-commit vs high-throughput take-first should be **named modes**, not tribal knowledge.
5. **Rename only when expected gain ≫ migration cost.** Keep `internal_behavior`, `try_peek`, `await_for_any!` if already distinctive.
6. **Aliases first for wild code.** Dual names for one major release; rustdoc banners; then deprecate.
7. **Doc vocabulary can lead the crate.** Agents read lessons before crates.io; rename docs/skills in lockstep with aliases.

---

## 3. Rename catalog

Each row: **Current → Proposed**, why LLMs fail, why the new name helps, migration.

### Top five (do these first — highest LLM leverage)

1. **`lock` → `acquire_guard`** on channels and `SteadyState` (aliases; never teach “mutex”).
2. **Doc phrase “Lock-first” → “Guard-first (bind-all-at-entry)”.**
3. **`simulated_behavior` → document/alias as `puppet_behavior`.**
4. **Wake `bool` → `#[must_use]` / `ReactorWake { Clean, Interrupted }`.**
5. **Explicit mode vocabulary: `DurableCommit` vs `ThroughputTake` + `outer reactor tick`.**

Everything else is secondary. Do **not** rename `internal_behavior`, `await_for_*`, or `try_peek`/`try_take` in the first migration wave.

### 3.1 Resource binding (highest priority)

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `SteadyRx::lock` / `SteadyTx::lock` / bundle `.lock()` | **`acquire_guard()`** (primary); rustdoc alias `bind()` | “Don’t hold locks across await” → unlock mid-LLM | “Guard” = held for instance; not a mutex | Alias `lock` → `acquire_guard` for 1+ major; deprecate `lock` |
| Doc phrase “Lock-first rule” | **“Guard-first (bind-all-at-entry)”** | Same mutex prior | Matches acquire_guard | Docs/skills only first |
| `StateGuard` / channel `MutexGuard` impls | Keep `StateGuard`; publicize **`ChannelGuard`** / document `RxGuard`/`TxGuard` aliases | Hidden that guard ≠ std mutex semantics | Explicit type story | Type aliases + docs |
| `SteadyState::lock(init)` | **`acquire_guard(init)`** | Same as channel lock | Parallel naming with channels | Alias |
| Comment “releases the lock” on panic | **“Drops the guard; rings retain messages”** | Implies data loss | Teaches durability | Docs only |

**Locked recommendation:** Do **not** name anything `Mutex`, `with_lock`, or `critical_section` for channel binding.

### 3.2 Actor anatomy

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `run` | **Keep `run`** | Low damage when documented as dispatcher | Familiar entry | None |
| `internal_behavior` | **Keep `internal_behavior`** | Costly rename; already unique | Distinct from Tokio | None — strengthen rustdoc: “DOMAIN LOOP — call this in unit tests” |
| `use_internal_behavior` | **`runs_domain_logic`** or keep + rustdoc | Negated thinking (“use” when false) | Boolean reads as production/unit path | Prefer **keep** + banner; optional alias |
| `simulated_behavior` | **`puppet_behavior`** (alias) | “Simulated” ≈ mock internals | “Puppet” matches StageManager mental model | Alias `simulated_behavior` |
| Doc “sacred switch” | Keep informal; formal: **Dispatcher branch** | — | — | Docs |

### 3.3 Shadow / Spotlight / context

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `SteadyActorShadow` | **Keep**; subtitle **GraphHandle** in rustdoc | Metaphor opaque | Dual label | Docs |
| `SteadyContext` alias | Document as deprecated synonym of Shadow | Confusion: two names | Prefer Shadow only in new code | Soft deprecate alias in docs |
| `into_spotlight` | **Keep**; rustdoc **“activate monitoring + channel metadata”** | Skipped carelessly | Explicit purpose | Docs |
| `SteadyActorSpotlight` | **Keep**; subtitle **ActiveActor** | — | — | Docs |

**Locked recommendation:** Do not rename Shadow/Spotlight—unique and searchable. Fix via rustdoc first lines.

### 3.4 Lazy vs Established channels

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `LazySteadyRx` / `LazySteadyTx` | **Keep Lazy\***; optional alias `BlueprintRx`/`BlueprintTx` | “Lazy” ≈ LazyLock | Blueprint = graph-time only | Optional aliases |
| Docs “Established” | Introduce types or aliases **`SteadyRx` = established** already; add rustdoc **`/// Established (hot-path) handle — clone from Lazy`** | Docs say Established; no type | Align docs ↔ types | Docs |
| “Clone to establish” | Phrase **“clone activates buffers on this thread”** | Clone seen as cheap Arc copy only | Teaches affinity | Docs + panic messages |

### 3.5 Progress / wake macros

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `await_for_all!` / `await_for_any!` | **Keep** | Already good | Intent + resource | None |
| Clean `bool` return | Type **`WakeResult { Clean, Interrupted }`** or `#[must_use]` bool wrapper **`ReactorWake`** | Ignored `clean` | Forces handling | Newtype + From\<bool\> |
| `await_for_all_or_proceed_upon!` | **Keep**; rustdoc “first arm OR all remaining” | Hard to parse | Example-first docs | Docs |
| “Single wake point” | Standard term **`outer reactor tick`** | Vague “loop top” | Matches phase lessons | Docs/skills |

### 3.6 Wait APIs

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `wait_avail` | **Keep** | Good | — | None |
| `wait_vacant` | **Keep** | Good | — | None |
| `wait_avail_index` / `wait_vacant_index` | **Keep**; deprecate bundle-wide waits harder | Bundle waits misused | Index = multiplex lane | Docs |
| `wait_avail_bundle` / `wait_vacant_bundle` | Finish deprecation → remove | Still found in old samples | Force index API | Remove after alias period |
| `wait_periodic` | Rename to **`wait_work_compensated_interval`** OR keep + rustdoc **“NOT for idle liveliness”** | Used as heartbeat | Name encodes semantics | Prefer **keep name** + severe rustdoc; mvp-ingest bans via CI |
| `wait_timeout` | **Keep**; rustdoc **“armed deadline only”** | Confused with periodic | Clear | Docs |
| `wait` | Rename to **`sleep_duration`** or **`wait_sleep`** | Confused with reactor wait | Not a channel wait | Alias |
| `wait_shutdown` | **Keep** | Good | — | None |
| `wait_empty` | **`wait_tx_drained`** | Ambiguous empty | TX drain explicit | Alias |

### 3.7 Peek / take / send

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `try_peek` / `advance_take_index` | **Keep**; package as **durable mode** | Skip peek | Mode docs | Docs + macros below |
| `try_take` | **Keep** | Used too early on durable paths | Mode docs | Docs |
| `try_send` / `send_async` | **Keep** | — | — | None |
| `SendSaturation::AwaitForRoom` | **Keep** (excellent name) | — | — | None |
| `SendSaturation::ReturnBlockedMsg` | Finish remove | Footgun | Force `try_send` | Remove |
| `SendOutcome` | **Keep**; rustdoc table for Blocked/Timeout/Closed | Match arms forgotten | — | Docs |
| `is_showstopper` | **`is_repeat_peek_poisoned`** alias or keep + rustdoc | Opaque | Poison/retry language | Prefer keep + rustdoc “N peeks without take → drop candidate” |

### 3.8 Shutdown / liveliness

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `is_running(\|\| veto)` | Rustdoc: **“returns false when veto refuses stop”** inverted clarity; optional rename of param to **`accept_shutdown`** | Closure polarity confusing | `accept_shutdown` matches return-true-to-stop | Rename param in docs/signature comment |
| `mark_closed` | **Keep** | Forgotten | — | Docs stress |
| `is_closed_and_empty` | **Keep** (excellent) | — | — | None |
| `i!(expr)` | **Keep**; rustdoc title **VetoEye** | Unknown | Links to unclean stop | Docs |
| `GraphLivelinessState::StoppedUncleanly` | **Keep** | — | — | None |
| `request_shutdown` | **Keep** | — | — | None |

### 3.9 State

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `SteadyState<S>` | **Keep** | Confused with framework name | rustdoc: “panic-surviving actor memory” | Docs |
| `new_state` / `new_persistent_state` | **Keep** | — | — | None |
| Update-after-success discipline | Doc term **`commit_state_after_io`** | Update before send | Explicit | Docs |

### 3.10 Bundles / girth

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| Girth / `GIRTH` | **Keep**; rustdoc **“const lane count”** | Exotic word | Lane count synonym | Docs: Girth = lane count |
| `SteadyRxBundle` | **Keep** | — | — | None |
| `wait_avail_vacant_index` | Rustdoc **deadlock if thresholds never pair** | Silent hang | Warning | Docs (already in steady-bundle-waits) |

### 3.11 Scheduling

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `ScheduleAs::SoloAct` | **Keep** | — | — | None |
| `MemberOf(&mut Troupe)` | **Keep**; fix spelling in docs **troupe** (lesson file is `troups`) | Typo drift | Consistency | Rename lesson file when convenient |
| `Troupe` / `TroupeGuard` | **Keep** | — | — | None |

### 3.12 Testing

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `GraphBuilder::for_testing` | Rustdoc **“defaults to puppet actors”** first line | Surprise hang | Explicit | Docs |
| `simulated_behavior` | See puppet alias above | — | — | Alias |
| `StageManager` | **Keep** (good metaphor) | — | — | None |
| `StageDirection::Echo` | **Keep** | — | — | None |
| `testing_send_all` | **Keep** | — | — | None |
| `with_test_pipeline_internal_behavior_names` | Shorten alias **`force_domain_logic_for`** | Unwieldy | Discoverable | Alias |

### 3.13 Side I/O / telemetry

| Current | Proposed | Why LLMs fail | Why new name helps | Migration |
|---------|----------|---------------|--------------------|-----------|
| `call_async` / `call_blocking` | **Keep** | — | — | None |
| `BlockingCallFuture::fetch` | **Keep**; rustdoc shutdown → `None` | Retry forgotten | — | Docs |
| `relay_stats` / `relay_stats_smartly` | Rustdoc **“is_running already relays — do not call in hot loop”** | Double relay / wait(frame) abuse | Banner | Docs |
| `yield_now` | Rustdoc **troupe fairness only — not idle** | Used as sleep | — | Docs |

### 3.14 Durable vs performant modes (new vocabulary)

| Current (implicit) | Proposed explicit terms | Why | Migration |
|--------------------|-------------------------|-----|-----------|
| Peek → work → take | **`DurableCommit` path** / rustdoc module `steady_state::durable` | LLMs mix with take-first | Docs + optional helper macros |
| Take → work (lesson 02B) | **`ThroughputTake` path** / `steady_state::throughput` | Same | Docs; never default for robust apps |
| Phase enum + one step/wake | **`OuterTick` / `bounded_phase_step`** | Unbounded drains | Docs + skill |

**Locked recommendation:** Do not invent a second actor trait. Add **documentation modules and example macros**:

```text
// Proposed (future) — illustrative only
durable_peek_commit!(actor, rx, |msg| { ... }); // expands to peek → work → advance/take
```

---

## 4. Small API tweaks (proposed, not implemented)

These are **shape** changes that make the correct path hard to miss. Ranked by expected LLM impact.

| ID | Tweak | Effect |
|----|-------|--------|
| T1 | `acquire_guard` aliases + rustdoc deny “mutex” language | Cuts #1 failure mode |
| T2 | `#[must_use]` on wake `bool` / `ReactorWake` newtype | Forces clean/dirty handling |
| T3 | `#[cfg(test)]` lint or clippy: `run(` inside `#[test]` without `force_domain` | Cuts puppet trap |
| T4 | Builder method `armed_timeout(Option<Duration>)` composing into `await_for_any!` | Makes idle-without-timer the default |
| T5 | Deprecate `wait_avail_bundle` / `wait_vacant_bundle` to hard error in next major | Forces index waits |
| T6 | Panic/message on Lazy used in wait: “clone LazySteady* before actor start” | Faster diagnose |
| T7 | `SendSaturation::ReturnBlockedMsg` removal | Eliminates wrong backpressure API |
| T8 | Example template crate `steady-actor-scaffold` with guard-first skeleton | Training data for agents |
| T9 | Dual rustdoc examples: DurableCommit vs ThroughputTake side-by-side | Mode clarity |
| T10 | Telemetry: assert/warn if `relay_stats*` called more than N/sec from actor body | Stops hot-loop relay |

---

## 5. Knob coverage checklist

Every Steady knob area below has a rename/doc action in §3–4 or an explicit **keep**.

| Area | Covered |
|------|---------|
| Anatomy `run` / `internal_behavior` / simulation | §3.2 |
| Shadow / Spotlight | §3.3 |
| Rx/Tx lock → guard | §3.1 |
| Lazy / Established | §3.4 |
| SteadyState | §3.9 |
| await_for_* / clean wake | §3.5 |
| wait_avail/vacant/index/periodic/timeout/shutdown | §3.6 |
| peek/take/send/saturation/outcome | §3.7 |
| Shutdown veto / i! / mark_closed | §3.8 |
| Showstopper / durable | §3.7, §3.14 |
| Bundles / girth | §3.10 |
| SoloAct / troupe | §3.11 |
| Testing / StageManager / puppet | §3.12 |
| call_async / call_blocking | §3.13 |
| Logging / relay_stats | §3.13 |
| Durable vs performant | §3.14 |
| Graph liveliness / unclean stop | §3.8 |
| Stream/Aeron (distributed) | Keep names; out of mvp-ingest agent focus — document “same guard-first” only |
| Feature flags (exec_*, telemetry_*) | Keep; lesson-00 already clear |

---

## 6. Wild-code migration strategy

1. **Phase 0 (now):** These markdown docs + deep audit against **current** names. No code renames.
2. **Phase 1 (crate):** Add method aliases (`acquire_guard` = `lock`), rustdoc banners, `ReactorWake` optional.
3. **Phase 2 (docs/skills/lessons):** Switch teaching vocabulary to guard-first / puppet / outer reactor tick; keep code samples compiling on aliases.
4. **Phase 3 (consumers):** mvp-ingest / steady-llm codemod `lock()` → `acquire_guard()` behind a tracked PR; CI allow both during transition.
5. **Phase 4:** Deprecate `lock` with rustc warning; one major later remove.

**Do not** force mvp-ingest onto new names until Phase 1 ships in the dependency they actually compile against.

---

## 7. What not to rename (pass-2 cull)

These were considered and **rejected** as low value or high confusion:

| Keep | Reason |
|------|--------|
| `internal_behavior` | Unique; rename churn across every actor; LLMs can learn it |
| `await_for_all!` / `await_for_any!` | Already optimal |
| `try_peek` / `try_take` / `try_send` | Align with Rust `try_*` |
| `SoloAct` | Memorable |
| `StageManager` | Good metaphor |
| `Girth` | Niche but taught; “lane count” as synonym enough |
| `SteadyState` | Framework brand; clarify in rustdoc instead |
| `wait_periodic` **name** | Renaming does not stop misuse; CI + rustdoc stronger; mvp bans |

---

## 8. Success criteria

After Phase 1–2 land:

1. An LLM given only the scaffold + rustdoc (no long manifesto) drafts an actor that scores **≥ 85** on [`deep-actor-audit.md`](deep-actor-audit.md) core section on first try.
2. Puppet-trap hangs in unit tests drop to near zero (lint or docs).
3. Agents stop proposing “release the lock before the LLM await.”
4. New contributors use **guard-first** / **outer reactor tick** language in reviews.

---

## 9. Cross-links

- Audit (current API): [`deep-actor-audit.md`](deep-actor-audit.md)
- TLDR: [`.clinerules/steady_state_tldr.md`](../../.clinerules/steady_state_tldr.md)
- Strict overlay: [`../steady-strict-contract.md`](../steady-strict-contract.md)
- Guards ≠ occupancy: [`../actors/lane-occupancy.md`](../actors/lane-occupancy.md)
- Skills index: [`.cursor/skills/steady-state-index/SKILL.md`](../../.cursor/skills/steady-state-index/SKILL.md)

---

## 10. 0.3.0 execution status

Verified against the tree on 2026-08-18; Phase 1 implemented on branch `guard-first-0.3.0`.

| Item | Status |
|------|--------|
| T1 `acquire_guard` aliases (extension trait for `SteadyRx`/`SteadyTx`; provided methods on bundle/stream bundle traits; inherent on `SteadyState` + builder context) | **Landed 0.3.0** |
| Rustdoc “mutex guards” / “Lock-first” purge → “Guard-first (bind-all-at-entry)” | **Landed 0.3.0** |
| T2 `#[must_use]` with teaching message on all `bool`/`Option`-returning `wait_*` / `relay_stats_periodic` methods | **Landed 0.3.0** |
| T2 `ReactorWake { Clean, Interrupted }` newtype | **Deferred to 0.4.0** (breaking: changes every wait signature) |
| T5 `wait_avail_bundle` / `wait_vacant_bundle` | **Deprecated `since = "0.3.0"`** (pre-existing); all in-tree examples migrated to `*_index`; removal in 0.4.0 |
| T7 `SendSaturation::ReturnBlockedMsg` | **Deprecated** (pre-existing); removal in 0.4.0 — deprecated and removed in the same major would skip the warning window |
| Examples (30 files) + codegen template `file_actor.txt` emit `acquire_guard` | **Landed 0.3.0** |
| Newtype wrappers for `SteadyRx`/`SteadyTx` (the only change that fully removes foreign `.lock()`) | **Deferred to 0.4.0** with `#[deprecated]` `lock()` forwarder |
| `SteadyState::lock(init)` / owned `lock()` deprecations | **Deferred to 0.4.0** (aliases only in 0.3.0, per Phase 1) |

Key implementation note: `SteadyRx<T>`/`SteadyTx<T>` are type aliases over `futures::lock::Mutex`, so single-channel `.lock()` is a **foreign** method — the 0.3.0 alias is delivered through the `SteadyChannelExt` extension trait (glob-exported), and full removal of the false friend requires the 0.4.0 newtype step.

---

## Revision / accuracy

| Pass | Date | Notes |
|------|------|-------|
| Draft | 2026-07-30 | Full catalog from vendor inventory + TLDR false friends |
| Pass 2 | 2026-07-30 | Culled `internal_behavior` / `wait_periodic` renames; locked `acquire_guard`; rejected mutex language; added **Top five** priority box |
| Pass 3 | 2026-07-30 | Knob checklist complete vs skills + vendor surface; wild-code phased migration explicit |
| Final | 2026-07-30 | Cross-checked against lane-occupancy (guards held across await = correct); Phase-2 ≠ unlock; companion audit uses **current** names only |
| 0.3.0 | 2026-08-18 | Phase 1 executed (§10): `acquire_guard` aliases via extension trait + owned surfaces, wake `#[must_use]`, example/codegen migration; newtypes + deprecations deferred to 0.4.0 |
