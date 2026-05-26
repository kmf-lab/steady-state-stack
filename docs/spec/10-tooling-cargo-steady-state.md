# cargo-steady-state tooling

**Who should read this:** Contributors to the CLI/codegen crate.

**See also:** `cargo-steady-state/src/`, CHANGELOG Unreleased tooling section.

---

ss[tooling.cargo-driver-strings]

Generated actors MUST emit `wait_avail` / `wait_vacant` for single-lane drivers and `wait_avail_bundle` / `wait_vacant_bundle` for multi-lane EventDriven/CapacityDriven paths per current templates.

**Tier:** 0 (CLI impl)

---

ss[tooling.cargo-percent-parse]

`extract_percent` and bundle percent parsing MUST handle decimal edge cases without panicking on short driver vectors.

**Tier:** 0

---

ss[tooling.cargo-bundle-codegen]

CapacityDriven bundle codegen MUST NOT read past the end of the driver vector (no `v[2]` when len == 2).

**Tier:** 0

---

ss[tooling.cargo-capacity-driven]

`build_driver_block` MUST cover `AtMostEvery`, `Other`, and `AtLeastEvery` + `EventDriven` combinations used in production templates.

**Tier:** 1

---

ss[tooling.cargo-index-wait-deferred]

Codegen MUST NOT emit `wait_*_index` until templates pin a `steady_state` release that exports those APIs (documented deferral).

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `tooling.cargo-driver-strings` | Driver string emission | 0 |
| `tooling.cargo-percent-parse` | Percent parsing | 0 |
| `tooling.cargo-bundle-codegen` | Safe bundle codegen | 0 |
| `tooling.cargo-capacity-driven` | CapacityDriven blocks | 1 |
| `tooling.cargo-index-wait-deferred` | Index wait deferral | 1 |
