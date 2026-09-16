# Lambda warm graph example

Demonstrates `GraphBuilder::for_lambda()`, the host bridge, and (with `--runtime`) the
blocking Runtime API client behind the `lambda` Cargo feature.

## Local demo (no AWS)

```bash
cargo run -p steady_state --example lambda_warm --features lambda
```

## Real Runtime API

Set by the Lambda execution environment. Cargo-lambda may set it for local invoke.

```bash
cargo run -p steady_state --example lambda_warm --features lambda -- --runtime
```

## Notes

- Dynamic troupe packing maps actors onto this sandbox’s CPU at `start`.
- Persist-before-park: call `StateGuard::persist()` before every await when durable.
- Do not enable Steady’s `tokio` feature for this path.
