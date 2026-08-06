
# Publisher Pod

## Overview

The publisher pod generates data, serializes it, and transmits it to the subscriber pod over Aeron. Built with the `steady_state` framework, it consists of a pipeline of actors designed for high throughput and reliability.

### Actors in the Publisher Pipeline

- **Heartbeat Actor**: Generates periodic "beats" to pace the system and synchronize with the subscriber.
- **Generator Actor**: Produces a high-rate stream of sequential numbers, simulating a data source.
- **Serialize Actor**: Batches data from the heartbeat and generator actors, serializes it into byte streams, and prepares it for transmission.
- **Publish Actor**: Sends the serialized data over Aeron to the subscriber.

### Architecture

The publisher’s actors are connected as follows:
```
[HEARTBEAT]      [GENERATOR]
    |                |
    |                |
    +-------+--------+
     |
[SERIALIZE]
     |
[STREAM BUNDLE]
     |
[PUBLISH]
     |
[AERON IPC/UDP]
```
- **Heartbeat Actor**: Sends timing signals (e.g., every 20 microseconds by default) to coordinate the system.
- **Generator Actor**: Generates batches of sequential numbers (1 million per beat by default).
- **Serialize Actor**: Combines and serializes data into a dual-channel stream (control and payload).
- **Publish Actor**: Transmits the serialized stream over Aeron.

### Aeron Configuration

The publisher uses Aeron for high-performance, low-latency communication with the subscriber. Aeron can be configured via command-line arguments:

- **IPC (Inter-Process Communication)**: For same-machine communication (default).
    - Example: `publish --comm-type ipc`
- **UDP**: For cross-machine communication.
    - Requires `--ip` and `--port` arguments.
    - Example: `publish --comm-type udp --ip 192.168.1.100 --port 40456`

The Aeron channel is defined in `pod/publish/src/main.rs` and defaults to IPC. To switch to UDP, modify the `AeronConfig` in the code or pass the appropriate arguments.

### Additional Configuration

Command-line arguments (parsed via `clap` in `pod/publish/src/arg.rs`):

- `--rate <microseconds>`: Rate of heartbeat generation (default: `20`).
- `--beats <number>`: Total number of beats to generate (default: `10000`).

### Telemetry

The publisher runs a telemetry server on `127.0.0.1:5551`, providing real-time monitoring of system health and performance metrics (e.g., channel fill levels, actor CPU usage).

## Usage

To run manually (without the runner):

1. Build: `cargo build --release --bin publish`
2. Run: `./target/release/publish --comm-type ipc`
3. Monitor telemetry at `http://127.0.0.1:5551`.

Typically, the runner launches the publisher automatically.
When reviewing the source code, look for //#!#// which demonstrate key ideas you need to know.
