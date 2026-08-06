
# Steady State Core and Runner

## Overview

The `steady_state` framework is a Rust library designed to build robust, high-performance actor systems for distributed applications. It powers the core functionality of this project and provides the following key features:

- **Actor Isolation**: Each actor runs in its own thread, ensuring failure isolation and simplifying debugging.
- **Persistent State**: Actors maintain state that persists across panics or restarts, ensuring data integrity.
- **High-Throughput Channels**: Efficient communication channels with built-in telemetry and triggers for monitoring system health.
- **Distributed Communication**: Integration with [Aeron](https://github.com/real-logic/aeron), a high-performance messaging library, for ultra-low latency communication between processes or machines.
- **Telemetry and Monitoring**: Built-in telemetry servers provide real-time insights into system performance and health.

The **runner** is a utility that orchestrates the deployment of the publisher and subscriber pods, which are built using the `steady_state` framework. It ensures that the distributed system starts up correctly and shuts down gracefully.

## Runner Functionality

The runner manages the lifecycle of the publisher and subscriber pods with the following steps:

1. **Builds the Pods** (optional):
    - If `USE_CUSTOM_CARGO_BUILD` is enabled, the runner builds the `publish` and `subscribe` binaries using Cargo.
    - Supports both debug and release profiles, configured via `BuildMode` in `runner/src/main.rs`.
    - If `CLEAN_BEFORE_BUILD` is true, it runs `cargo clean` before building.

2. **Launches the Pods**:
    - Spawns the publisher and subscriber processes in sequence, with a configurable delay (`LAUNCH_DELAY_SECS`) between launches to ensure proper initialization.
    - Locates binaries in `./target/release/` or `./target/debug/` based on the build mode.

3. **Handles Shutdown**:
    - Sets up a Ctrl-C handler to terminate all running pods gracefully, ensuring no orphaned processes remain.

### Configuration

The runner’s behavior is controlled by constants in `runner/src/main.rs`:

- `USE_CUSTOM_CARGO_BUILD`: If `true`, builds pods before launching (default: `true`).
- `CLEAN_BEFORE_BUILD`: If `true`, cleans the build directory before building (default: `false`).
- `LAUNCH_DELAY_SECS`: Delay in seconds between pod launches (default: `3`).

### Aeron Communication

The publisher and subscriber pods communicate over Aeron, a high-performance messaging system. The runner does not directly configure Aeron but launches the pods with their respective Aeron settings:

- **Publisher**: Sends data over Aeron (IPC or UDP) to the subscriber.
- **Subscriber**: Receives data from the publisher via Aeron.

Aeron configuration (e.g., IPC vs. UDP, IP, port) is passed to the pods via command-line arguments. For details, see the [Publisher README](#readme-2-publisher) and [Server README](#readme-3-server).

### Setup Aeron
To make use of aeron the aeron media driver must be installed on your machine in order to route messages.

All the details for installation are found in the platform-specific Readme found in install_aeron folder.

The default setup is for greater throughput and should be able to saturate a 2.5Gb ethernet connection.
You can modify the installer to use smaller buffer if you want to minimize your memory usage.
With the right settings, the aeron media driver can be under 20MB.
If you need configurations for 10Gb or faster you should review [aeron.io/docs](https://aeron.io/docs/aeron/media-driver/)

## Usage

To run the system:

1. Ensure Rust and Cargo are installed.
2. Navigate to the `runner` directory.
3. Run `cargo run` to build and launch the pods.
4. Use Ctrl-C to terminate all pods cleanly.

The runner logs build and launch progress, including build times stored in temporary files for reference.
When reviewing the source code, look for //#!#// which demonstrate key ideas you need to know.


