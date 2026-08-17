# Self-hosted GitHub runner for Aeron Gate C

Use this guide to configure a machine that runs [`.github/workflows/aeron-integration.yml`](../.github/workflows/aeron-integration.yml) with the **full transport matrix** (IPC + UDP + multicast).

## Labels

Register the runner with GitHub labels:

- `self-hosted`
- `aeron`

The workflow job `aeron-ipc-self-hosted` requires both.

## Media driver (same user as the runner)

The runner process user must match the Aeron IPC identity:

```bash
cd core/routing_service/aeron
sudo bash install_aeronmd.sh "$USER"
systemctl status aeronmd   # or: docker ps --filter name=aeronmd
```

Details: [core/routing_service/aeron/README_linux.md](../core/routing_service/aeron/README_linux.md).

## UDP / multicast host prep (full matrix)

Before release sign-off or `SS_AERON_MATRIX=full`:

1. Apply loopback/socket tuning (optional but recommended for UDP scenarios):

   ```bash
   sudo bash core/routing_service/aeron/update_sysctl.sh
   ```

2. Ensure loopback UDP is not blocked by host firewall.

3. Multicast scenarios use group `224.0.1.1` on loopback; confirm the OS allows multicast on `lo`.

## Smoke (one scenario)

```bash
docker restart aeronmd && sleep 15

SS_AERON_GATE_C=1 SS_AERON_SCENARIO=ipc_single_one SS_AERON_REQUIRED=1 \
  cargo test -p steady_state \
  --test aeron_integration_suite -- --nocapture
```

## Release sign-off (full matrix, 17 scenarios)

```bash
docker restart aeronmd && sleep 15
bash scripts/run-aeron-release-signoff.sh
```

Flake check (3 consecutive passes):

```bash
bash scripts/run-aeron-flake-check.sh
```

## Examples smoke (optional)

After Gate C is green:

```bash
bash scripts/smoke-aeron-examples.sh
```

## CI workflow

Trigger **Aeron integration** → job **aeron-ipc-self-hosted** (`workflow_dispatch`). It runs `scripts/run-aeron-release-signoff.sh` with `SS_AERON_RELEASE=1` and uploads `/tmp/aeron-integration.log` on failure.

The `ubuntu-latest` job only validates that the integration script fails fast when no driver is present (not a Gate C pass).
