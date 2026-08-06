
# Subscriber Pod

## Overview

The subscriber pod, receives data from the publisher over Aeron, deserializes it, processes it using the FizzBuzz algorithm, and logs the results. Built with the `steady_state` framework, it operates as an independent process in the distributed system.

### Actors in the Subscriber Pipeline

- **Aeron Actor**: Receives the dual-channel stream (control and payload) from the publisher via Aeron.
- **Deserialize Actor**: Deserializes byte streams into `u64` values, checks sequence integrity, and splits into heartbeat and generator channels.
- **Worker Actor**: Applies FizzBuzz logic to the generator values, synchronized by heartbeats.
- **Logger Actor**: Logs statistics about the processed FizzBuzz results.

### Architecture

The subscriber’s actors are connected as follows:
```
         [AERON]
           |
      (STREAM BUNDLE)
           |
      [DESERIALIZE]
       /        \  
(HEARTBEAT) (GENERATOR)
   |             |
 +------[WORKER]------+
             |
         [LOGGER]
```

- **Aeron Actor**: Receives serialized data from the publisher.
- **Deserialize Actor**: Converts bytes back into `u64` values and ensures sequence correctness.
- **Worker Actor**: Processes batches of 1 million generator values per heartbeat, producing FizzBuzz results.
- **Logger Actor**: Aggregates and logs counts of "Fizz," "Buzz," "FizzBuzz," and plain values.

### Aeron Configuration

The subscriber uses Aeron to receive data from the publisher. Configuration matches the publisher’s settings:

- **IPC**: For same-machine communication (default).
    - Example: `subscribe --comm-type ipc`
- **UDP**: For cross-machine communication.
    - Requires `--ip` and `--port` arguments matching the publisher.
    - Example: `subscribe --comm-type udp --ip 192.168.1.100 --port 40456`

The Aeron channel is defined in `pod/subscribe/src/main.rs` and defaults to IPC. Ensure the subscriber’s configuration aligns with the publisher’s for successful communication.

### Additional Configuration

Command-line arguments (parsed via `clap` in `pod/subscribe/src/arg.rs`):

- `--rate <microseconds>`: Expected heartbeat rate (default: `20`, must match publisher).
- `--beats <number>`: Expected number of beats (default: `10000`, must match publisher).

### Telemetry

The subscriber runs a telemetry server on `127.0.0.1:5552`, providing real-time monitoring of system health and performance metrics.

## Usage

To run manually (without the runner):

1. Build: `cargo build --release --bin subscribe`
2. Run: `./target/release/subscribe --comm-type ipc`
3. Monitor telemetry at `http://127.0.0.1:5552`.

Typically, the runner launches the subscriber after the publisher.
When reviewing the source code, look for //#!#// which demonstrate key ideas you need to know.
