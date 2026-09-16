# Coverage-guided fuzz (cargo-fuzz)

Parse and protocol edges are fuzzed with **libFuzzer** via `cargo-fuzz`. Actor graphs and live Aeron are **out of scope** for this first campaign (too slow / hang-prone).

## Requirements

- Nightly Rust: `rustup toolchain install nightly`
- clang (libFuzzer link)
- `cargo install cargo-fuzz`

## Run

From the repository root:

```bash
bash scripts/run-fuzz.sh
```

Defaults to **20 seconds per target** (`SS_FUZZ_SECONDS`). Skip when nightly or clang is missing:

```bash
SS_SKIP_FUZZ=1 bash scripts/run-fuzz.sh
```

`pre-publish.sh` calls the same script after Gate A. Seed corpora live under `core/fuzz/corpus/<target>/`. Crashes land in `core/fuzz/artifacts/` (gitignored).

## Targets

| Target | Surface | Feature path |
|--------|---------|--------------|
| `aeron_channel_uri` | `Channel::cstring()` URI build | public API |
| `fast_protocol_packed` | FAST signed/unsigned long codec | `--features fuzzing` |
| `cgroup_quota` | cgroup v1/v2 CPU quota parse | `--features fuzzing` |
| `pack_even_split` | even-split / empty-edge pack | `--features fuzzing` |

Single target:

```bash
cd core && cargo +nightly fuzz run aeron_channel_uri -- -max_total_time=60
```

## Campaign log

| Date | Seconds / target | Crashes | Notes |
|------|------------------|---------|-------|
| 2026-09-10 | 20 | (see run output) | Initial four-target program; local/pre-publish only |

Update this table after each campaign. A crash is a defect: fix the production code, then re-run. Do not weaken the harness to hide `CString::new` failures on `Channel::cstring`.
