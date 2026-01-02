
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Lesson: Testing and Simulation—Building Bulletproof Actor Graphs                                                                                         ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

In high-concurrency systems, traditional unit testing often fails to catch race conditions, deadlocks, and wiring errors. Steady State solves this by
providing a dual-layered testing architecture: Actor Simulation and Graph Integration via StageManager.

This lesson explains how to use these tools to verify your system from individual logic blocks up to the full distributed graph.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

1. The Testing Hierarchy

Steady State distinguishes between two types of tests. You need both to achieve high confidence.


  Feature     Actor Simulation (Unit)                                Graph Integration (Full)
 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Scope       A single actor in isolation.                           The entire graph (all actors + channels).
  Mechanism   actor.simulated_behavior()                             graph.stage_manager()
  Focus       Does this actor handle its inputs/outputs correctly?   Do actors talk to each other correctly?
  Speed       Ultra-fast (no real I/O).                              Fast (uses real memory-based channels).


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

2. Level 1: Actor Simulation (IntoSimRunner)

When you write an actor, you often want to test its internal_behavior without spinning up a whole graph. Steady State actors have a use_internal_behavior
flag. In tests, this is usually false, triggering simulated_behavior.

How it Works

The simulated_behavior method takes a list of "Sim Runners" (usually your Rx/Tx channels). It turns the actor into a "puppet" that can be controlled via a
side-channel.


pub async fn run(context: SteadyActorShadow, rx: SteadyRx<Msg>, tx: SteadyTx<Msg>) -> Result<(), Box<dyn Error>> {
    let mut actor = context.into_spotlight([&rx], [&tx]);

    if actor.use_internal_behavior {
        internal_behavior(actor, rx, tx).await
    } else {
        // In test mode, the actor just waits for commands to send/receive
        actor.simulated_behavior(vec![&rx, &tx]).await
    }
}


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

3. Level 2: The StageManager (Integration)

The StageManager is the "God-mode" controller for your graph. It is only available when the graph is built using GraphBuilder::for_testing().

The Side-Channel Architecture

When a graph is in testing mode, every actor is automatically assigned a Side-Channel.

 1 The Test Thread uses the StageManager to send commands (e.g., "Send this message into the graph").
 2 The Actor uses a SideChannelResponder to receive and execute those commands.

Building a Full Graph Test

To test a full graph, use the SteadyRunner::test_build() pattern. This ensures the test runs with a proper stack size and isolated environment.


#[test]
fn test_full_system_flow() -> Result<(), Box<dyn Error>> {
    SteadyRunner::test_build().run((), |mut graph| {
        // 1. Build your real graph logic
        let (tx, rx) = graph.channel_builder().build::<MyMsg>();
        graph.actor_builder().with_name("Worker").build(move |c| worker::run(c, rx), SoloAct);

        // 2. Start the graph
        graph.start();

        // 3. Use StageManager to interact with the "Worker" actor
        let stage = graph.stage_manager();

        // Command: Tell the "Worker" to expect a specific message
        stage.actor_perform("Worker", StageWaitFor::Message(MyMsg::default(), Duration::from_secs(1)))?;

        // 4. Shutdown
        drop(stage); // Must drop guard before shutdown
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(2))
    })
}


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

4. The SideChannelResponder

For the StageManager to work, the actor must be listening. This is handled automatically if you use simulated_behavior, but you can also use it inside your
internal_behavior for "Live Integration Testing."

Common Commands

 • StageDirection::Echo(msg): Tells an actor to send msg out of its Tx channel.
 • StageWaitFor::Message(msg, timeout): Tells an actor to wait until it receives msg on its Rx channel.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

5. Why This is Important: The "Wiring" Problem

In complex systems, the most common bugs are not in the logic, but in the wiring:

 • Actor A sends TypeX, but Actor B expects TypeY.
 • Channel capacity is too small, causing a deadlock.
 • Shutdown logic is missing a mark_closed() call, causing the graph to hang.

Full Graph Tests catch these because they use the real ChannelBuilder and is_running loops. If your wiring is wrong, the StageManager commands will time
out, and the test will fail.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

6. Summary Best Practices

 1 Always provide a simulation branch: Every run function should have an if actor.use_internal_behavior check.
 2 Name your actors: Use .with_name("Processor") in your builder. The StageManager relies on these names to find the side-channels.
 3 Drop the StageManager Guard: The StageManager holds a lock on the graph's backplane. You must drop it (or let it go out of scope) before calling
   graph.block_until_stopped().
 4 Use i! in tests: Simulation relies on the is_running loop. If you don't use i!, you lose the ability to see why a test hung in the telemetry.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Appendix: Common Testing Scenarios                                                                                                                       ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

1. Injecting Data into the Middle of a Graph

If you want to test "Actor B" but don't want to run "Actor A," use the StageManager to tell Actor B's input channel to "Echo" a message.


// Tell the "Input" actor to push a message into the graph
stage.actor_perform("Input", StageDirection::Echo(MyMsg { data: 42 }))?;


2. Verifying Output at the Edge

To verify that the final actor in your chain produced the right result:


// Tell the "Output" actor to wait for the expected result
stage.actor_perform("Output", StageWaitFor::Message(ExpectedMsg { status: 200 }, Duration::from_secs(1)))?;


3. Testing with Suffixes (Troupes)

If you have a troupe of 10 "Worker" actors, use the suffix to target a specific one.


// Target the 4th worker in the troupe
stage.actor_perform_with_suffix("Worker", 4, StageDirection::Echo(msg))?;


4. Mocking Aeron (Distributed)

Aeron requires a running Media Driver. In unit tests, you can avoid this by using never_simulate(false) and simulated_behavior. This allows you to test your
Aeron logic as if it were a local memory channel.


// In your test graph setup
graph.actor_builder()
     .with_name("AeronPublisher")
     .never_simulate(false) // Allow simulation
     .build(..., SoloAct);


5. The "God-Mode" Side Channel

You can define custom StageAction implementations to trigger internal state changes in your actors that aren't possible via normal channels (e.g., "Reset
all internal counters").


// Custom action implementation
impl StageAction for ResetCounters {}

// Inside actor
responder.respond_with(|msg, actor| {
    if msg.is::<ResetCounters>() {
        state.counters.clear();
        Some(Box::new("ok"))
    } else {
        None
    }
}, &mut actor);
