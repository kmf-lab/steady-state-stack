# Graph and shutdown

**Who should read this:** Graph builders, integration tests, and shutdown debugging.

**See also:** `graph_liveliness.rs`, [Actor lifecycle](../actor_lifecycle.md).

---

ss[graph.for-testing]

`GraphBuilder::for_testing` MUST configure graphs for unit/integration tests (including `use_internal_behavior` semantics documented in testing spec).

**Tier:** 0

---

ss[graph.shutdown.veto]

When an actor’s veto closure returns `false` during shutdown, the graph MUST record a veto vote and MUST NOT complete clean shutdown until resolved or timed out.

**Tier:** 0

---

ss[graph.shutdown.accept]

When the veto closure returns `true`, the actor’s vote MUST favor shutdown and `is_running` MUST eventually return `Some(false)`.

**Tier:** 0

---

ss[graph.block-until-stopped]

`block_until_stopped` MUST wait until all voters agree or timeout/unclean stop is reported.

**Tier:** 0

---

ss[graph.request-shutdown]

`request_shutdown` MUST transition liveliness to stop-requested and MUST fire one-shot shutdown notifications to actors.

**Tier:** 0

---

ss[graph.liveliness-voters]

Every running actor MUST be registered as a shutdown voter while active and MUST be removed or auto-voted when marked dead.

**Tier:** 0

---

ss[graph.panic-restart]

Actor panic MUST trigger restart path without losing channel contents; regeneration counter MUST increment.

**Tier:** 0

---

ss[graph.actor-identity]

Shutdown votes MUST be keyed by stable `ActorIdentity` for traceability and veto reporting.

**Tier:** 1

---

ss[graph.troupes]

Troupe execution MUST yield cooperatively (`yield_now` in `await_for_all!`) so nested graphs do not spin.

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `graph.for-testing` | Test graph builder | 0 |
| `graph.shutdown.veto` | Veto blocks clean stop | 0 |
| `graph.shutdown.accept` | Accept advances shutdown | 0 |
| `graph.block-until-stopped` | Wait for completion | 0 |
| `graph.request-shutdown` | Initiate stop | 0 |
| `graph.liveliness-voters` | Voter registration | 0 |
| `graph.panic-restart` | Panic → restart | 0 |
| `graph.actor-identity` | Vote identity | 1 |
| `graph.troupes` | Cooperative troupes | 1 |
