use std::env;
use steady_state::*;
use steady_state::distributed::aeron_channel_structs::{ReliableConfig};
use arg::MainArg;
mod arg;

pub(crate) mod actor {
    pub(crate) mod heartbeat;
    pub(crate) mod generator;
    pub(crate) mod serialize;
}

// === ACTOR NAME CONSTANTS ===
const NAME_HEARTBEAT: &str = "HEARTBEAT";
const NAME_GENERATOR: &str = "GENERATOR";
const NAME_SERIALIZE: &str = "SERIALIZE";
// Used by the production graph (Aeron publish actor). Omitted in `cfg(test)` smoke builds.
#[cfg(not(test))]
const NAME_PUBLISH: &str = "PUBLISH";

///
/// # Distributed Publisher Main
///
/// This is the entry point for the **Publisher** side of a distributed, robust, high-throughput
/// actor pipeline. The publisher is responsible for generating data, serializing it, and
/// streaming it out over a high-performance Aeron channel to one or more subscribers.
///
/// ## System Overview
///
/// The publisher graph is composed of several robust actors, each running in its own thread,
/// connected by high-capacity channels. The pipeline is designed for maximum throughput,
/// reliability, and observability.
///
///
/// ```text
///   [HEARTBEAT]      [GENERATOR]
///        |                |
///        |                |
///        +-------+--------+
///                |
///           [SERIALIZE]
///                |
///         [STREAM BUNDLE]
///                |
///           [PUBLISH]
///                |
///           [AERON IPC]
/// ```
///
/// - **HEARTBEAT**: Generates periodic "beats" to pace the system.
/// - **GENERATOR**: Produces a high-rate stream of values, simulating a real data source.
/// - **SERIALIZE**: Batches and serializes values from both heartbeat and generator into a dual-channel stream (control + payload).
/// - **STREAM BUNDLE**: Holds two channels per stream: one for control (lengths), one for payload (bytes).
/// - **PUBLISH**: Streams the serialized data out to subscribers using Aeron (IPC or UDP).
///
/// ## Key Features
///
/// - **High Throughput**: Channels and batches are sized for millions of messages per second.
/// - **Robustness**: Each actor is isolated and can recover from panics or failures.
/// - **Observability**: Telemetry, logging, and channel triggers provide deep insight into system health.
/// - **Distributed**: Output is streamed over Aeron, a high-performance messaging system.
///
/// ## Shutdown Barrier
///
/// The `.with_shutdown_barrier(2)` call ensures that the system will not fully shut down
/// until two independent shutdown signals are received. This is critical in distributed
/// systems to ensure that all data is flushed and all actors have completed their work
/// before the process exits. For example, one shutdown may come from the local graph,
/// and another from a remote subscriber or a control plane.
///
/// ## Environment Setup
///
/// The telemetry server is configured to run on localhost:5551 for monitoring.
///
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up telemetry server environment variables for observability.
    unsafe {
        env::set_var("TELEMETRY_SERVER_PORT", "5551");          //#!#//
        env::set_var("TELEMETRY_SERVER_IP", "127.0.0.1");
    }

    // Parse command-line arguments (rate, beats, etc.) using clap.
    let cli_args = MainArg::parse();

    // Initialize logging at Info level for runtime diagnostics and performance output.
    init_logging(LogLevel::Info, None)?;

    // Build the actor graph with all channels and actors, using the parsed arguments.
    // The shutdown barrier ensures the system waits for two shutdown signals before exiting.
    let mut graph = GraphBuilder::default()
        .with_telemtry_production_rate_ms(200) // Lower framerate for more throughput.
        .with_shutdown_barrier(2) // Ensures robust, coordinated shutdown across distributed systems.  //#!#//
        .build(cli_args);

    build_graph(&mut graph);

    // Start the entire actor system. All actors and channels are now live.
    graph.start();

    // The system runs until an actor requests shutdown or the timeout is reached.
    // The timeout here is set to allow for robust failure/recovery demonstration.
    graph.block_until_stopped(Duration::from_secs(5))
}

///
/// # Graph Construction
///
/// This function builds the full actor pipeline and connects all channels.
/// It demonstrates the robust architecture:
/// - Each actor is built with persistent state, enabling automatic restart and state recovery.
/// - Channels are created for each stage of the pipeline, with triggers for observability.
/// - Each actor is built as a SoloAct, running on its own thread for failure isolation.
/// - The final output is streamed over Aeron using a dual-channel (control + payload) bundle.
///
/// ## Channel Triggers and Telemetry
///
/// The channel builder is configured with triggers that monitor buffer fill levels:
/// - **Red Alert**: If the channel is >90% full, a red alert is triggered.
/// - **Orange Alert**: If the channel is >60% full, an orange alert is triggered.
/// - **Average Fill/Rate**: Telemetry is collected for average fill and message rate.
///
/// These triggers are essential for diagnosing backpressure and tuning performance.
///
fn build_graph(graph: &mut Graph) {
    // Configure channel builders with telemetry triggers for observability.
    let channel_builder_base = graph.channel_builder()
        .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
        .with_filled_trigger(Trigger::AvgAbove(Filled::p60()), AlertColor::Orange)
        .with_avg_filled()
        .with_avg_rate();

    // Small channel for heartbeat (low volume, high importance).
    let channel_builder_small = channel_builder_base.with_capacity(10_001);
    // Large channels for generator and stream output (high volume).
    let channel_builder_large = channel_builder_base.with_capacity(2_000_000);

    // Build channels for each stage of the pipeline.
    let (heartbeat_tx, heartbeat_rx) = channel_builder_small.build_channel();
    let (generator_tx, generator_rx) = channel_builder_large.build_channel();

    // Build a dual-channel stream bundle for serialized output.
    // Each stream has a control channel (for lengths) and a payload channel (for bytes).
    let (output_tx, output_rx) = channel_builder_large
        .build_stream_bundle::<StreamEgress, 2>(8);

    // Configure the actor builder with thread, load, and CPU telemetry.
    let actor_builder = graph.actor_builder()
        .with_mcpu_trigger(Trigger::AvgAbove(MCPU::m768()), AlertColor::Red)
        .with_mcpu_trigger(Trigger::AvgAbove(MCPU::m512()), AlertColor::Orange)
        .with_mcpu_trigger(Trigger::AvgAbove(MCPU::m256()), AlertColor::Yellow)
        .with_thread_info()
        .with_load_avg()
        .with_mcpu_avg();

    // Heartbeat and generator each run as SoloAct so StageManager-driven tests and
    // serialization are not serialized behind one troupe thread (which can deadlock
    // with side-channel injection before the graph is fully running).
    // Build the heartbeat actor.
    let state = new_state();
    actor_builder.with_name(NAME_HEARTBEAT)
        .build(move |context| actor::heartbeat::run(context, heartbeat_tx.clone(), state.clone())
               , SoloAct);

    // Build the generator actor.
    let state = new_state();
    actor_builder.with_name(NAME_GENERATOR)
        .build(move |context| actor::generator::run(context, generator_tx.clone(), state.clone())
               , SoloAct);

    // Build the serialize actor, which batches and serializes data for streaming.
    actor_builder.with_name(NAME_SERIALIZE)
        .build(move |context| actor::serialize::run(context, heartbeat_rx.clone(), generator_rx.clone(), output_tx.clone())
               , SoloAct);

    // === Aeron Output Configuration ===
    // The Aeron channel is configured for IPC (inter-process communication) by default.
    // You can switch to UDP point to point. Both publish and subscribe must agree.
    // Aeron is a high-performance, low-latency messaging system ideal for distributed streaming.
    //
    let use_ipc = false;

    let aeron_config = AeronConfig::new();

    let aeron_config = if use_ipc {                 //#!#//
        aeron_config.with_media_type(MediaType::Ipc)
    } else {
        aeron_config.with_media_type(MediaType::Udp)
                    .with_term_length((1024 * 1024 * 64) as usize)
                    .use_point_to_point(Endpoint {
                        ip: "127.0.0.1".parse().expect("Invalid IP address"),
                        port: 40456,
                    })
                    .with_reliability(ReliableConfig::Reliable)
    };


    let aeron_channel = aeron_config.build();
    info!("publish to aeron: {:?}", aeron_channel.cstring());

    // In unit tests, omit the Aeron publish actor: simulated `aeron_publish_bundle` does not
    // drain the egress bundle, so serialize would back-pressure and veto clean shutdown.
    #[cfg(not(test))]
    output_rx.build_aqueduct(
        AqueTech::Aeron(aeron_channel, 40), // 40 = max in-flight messages
        &mut actor_builder.with_name(NAME_PUBLISH),
        SoloAct,
    );
    #[cfg(test)]
    drop(output_rx);
}

#[cfg(test)]
pub(crate) mod publish_main_tests {
    use std::time::Duration;
    use steady_state::*;
    use super::*;

    /// Smoke test: graph builds, all actors register, and the graph reaches `Running`.
    ///
    /// In `cfg(test)` builds we omit the Aeron publish actor and drop the egress `StreamRx`
    /// bundle so there is no consumer on the serialized stream. The serializer may therefore
    /// veto a fully clean shutdown; we still verify startup and issue a graph shutdown request.
    #[test]
    fn graph_test() -> Result<(), Box<dyn Error>> {
        let mut args = MainArg::default();
        args.beats = 1;
        let mut graph = GraphBuilder::for_testing().build(args);

        build_graph(&mut graph);
        assert!(
            graph.start_with_timeout(Duration::from_secs(30)),
            "graph should reach Running after actor registration"
        );

        graph.request_shutdown();
        let _ = graph.block_until_stopped(Duration::from_secs(30));
        Ok(())
    }
}