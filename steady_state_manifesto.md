# The Steady State Manifesto

## I. The Core Tenets (Executive Summary)

1.  **Mechanical Sympathy**: Design for the hardware. Minimize context switches, cache misses, and unnecessary CPU cycles.
2.  **The Pull-Reactor Model**: Progress is driven by the consumer's intent and the producer's capacity, not by arbitrary external pushes.
3.  **Structural Hierarchy**: Strict separation between orchestration (`run`) and domain logic (`internal_behavior`) to facilitate simulation and testing.
4.  **Lock-First Resource Contract**: Acquire all resource guards asynchronously at the start of execution to eliminate hot-loop overhead and ensure type safety.
5.  **Cooperative Liveliness**: Shutdown is a negotiated agreement. Actors veto termination if work remains, ensuring data integrity.
6.  **Lazy-to-Established Lifecycle**: Channels are born "Lazy" in the Graph/Test layer and become "Established" upon cloning to the actor's thread.
7.  **Single Point of Wake-up**: Consolidate all wait conditions into a single macro-driven expression to prevent spin-locking and CPU saturation.
8.  **Zero-Copy Discipline**: Treat zero-copy as a semantic contract for ordering and memory locality, not just a micro-optimization.
9.  **Explicit Ownership**: The Graph (Main) owns the system; actors operate on clones, ensuring the system survives individual failures.

---

## II. Detailed Philosophies and Idioms

### 1. Mechanical Sympathy and the Pull-Reactor
Steady State rejects the "black box" approach of modern async runtimes. We do not treat the CPU as an infinite resource. Mechanical sympathy means understanding that every abstraction has a cost. 

The framework operates as a **Pull-Reactor**. In traditional push systems, producers overwhelm consumers, leading to complex buffer management and unpredictable latency. In Steady State, an actor only moves forward when it has the **intent** to act and the **resource** is available. If an actor is idle, it should consume 0% CPU. This is achieved by yielding to the reactor until a specific, registered condition is met.

### 2. The Actor Anatomy: `run` vs. `internal_behavior`
Every actor must implement a two-tier execution structure. This is non-negotiable for the sake of the simulation engine.

*   **The `run` Method**: This is the entry point called by the Graph. Its primary responsibility is **routing and conversion**. 
    *   It determines if the actor should run in "Production" mode or "Simulation" mode.
    *   It converts the `SteadyActorShadow` (the handle held by the Graph) into a `SteadyActorSpotlight` (the active execution context).
    *   It handles the telemetry setup, ensuring that the actor's performance is visible to the system before logic begins.
*   **The `internal_behavior` Method**: This is the sanctuary of domain logic. 
    *   It is the primary target for unit testing.
    *   It is isolated from the complexities of the Graph's orchestration.
    *   By testing `internal_behavior` directly, we can verify actor logic in a deterministic vacuum.

### 3. The Resource Contract: The "Lock-First" Rule
One of the most distinct idioms in Steady State is the requirement to lock all channels and state objects at the very beginning of `internal_behavior`.

*   **Async Acquisition**: All locks are acquired once using `await`.
*   **Performance**: By holding these locks for the entire duration of the actor's execution, we move the cost of synchronization out of the processing loop. The "hot loop" becomes a series of direct memory accesses through the guards.
*   **Type Safety**: Locking transforms a `SteadyRx<T>` (a handle) into a `RxGuard<T>` (a resource). This ensures that the compiler prevents the actor from attempting to use uninitialized or unacquired resources.
*   **Panic Recovery**: If an actor panics, the guards are dropped, and the locks are released. The supervisor then clones fresh handles from the Main thread, and the new instance re-acquires the locks, ensuring a clean slate.

### 4. The Liveliness Contract: `is_running` and the Veto
Shutdown in a distributed actor system is often messy. Steady State solves this through the `is_running` method and its veto closure.

*   **The Veto**: When the Graph requests a shutdown, `is_running` executes a closure provided by the actor. If the closure returns `false`, the shutdown is vetoed.
*   **Veto Logic**: An actor must veto if:
    1.  Incoming channels still contain unprocessed messages.
    2.  Internal state machines are in a "critical section" that cannot be safely interrupted.
*   **Short-Circuiting**: The closure must be simple and fast. It should check `rx.is_closed_and_empty()` first. If a channel is not closed and empty, the actor has work to do and must stay alive.

### 5. Channel Lifecycle: Lazy vs. Established
To maintain strict thread affinity and prevent race conditions during initialization, channels follow a specific lifecycle.

*   **Lazy Channels**: Created by the `GraphBuilder` or in unit tests. These are "blueprints" that contain configuration but no active buffers.
*   **Establishment via Clone**: When the Main thread calls `clone()` on a Lazy channel to pass it to an actor's `run` method, the channel is "established." This ensures the underlying buffers and wakers are initialized on the OS thread that will actually perform the work.
*   **Signature Discipline**: `internal_behavior` must never accept `Lazy` types. It only deals with established, lockable handles.

### 6. The Progress Loop: Wake-up Macros
Steady State actors are state machines that sleep until they are needed. We use the `await_for_all!` and `await_for_any!` macros to manage this.

*   **Single Point of Wake-up**: All conditions (channel capacity, data availability, timers) should be consolidated at the top of the `is_running` loop.
*   **Clean vs. Dirty Wake-ups**: These macros return a boolean. 
    *   `true`: The conditions were met. Proceed with work.
    *   `false`: The wait was interrupted by a shutdown signal. This is the cue to check the veto logic or wrap up.
*   **State Machine Discipline**: If an actor uses an internal state machine, every branch must eventually hit an `await` or a yield. No branch is allowed to loop back to the top without giving the reactor a chance to manage the CPU.

### 7. Testing and Mocking
Unit testing in Steady State is a first-class citizen. 
*   **Mocking the Graph**: In a unit test, the test runner acts as the "Main" thread. 
*   **Direct Interaction**: Unlike production code, unit tests are encouraged to use the `testing_send_all` and `testing_take_all` methods found on `Lazy` channels. This allows the test to inject state and verify outputs synchronously before and after the actor's `internal_behavior` runs.

### 8. Zero-Copy and Protocol Integrity
Zero-copy is not just about speed; it is about **ownership**. By using slice-based peeking, an actor can inspect data without moving it. This ensures that the sequence of messages (the protocol) remains intact until the actor explicitly "takes" the data. Violating this by copying data out of order is considered a "broken protocol" and is a fundamental failure of the actor's design.

---

## III. Philosophical Conclusion

Steady State is an exercise in **Timeless Engineering**. It is built on the belief that software should be explicit, deterministic, and kind to the hardware it inhabits. We favor simplicity over cleverness and invariants over intuition. 

A "Steady" system is one that does not spike, does not starve, and does not surprise. By following these idioms, we build systems that are not just fast, but **durable**—capable of running for months at 0% idle CPU, waking only with intent, and shutting down only when the work is truly done. We are not just writing code; we are crafting a collaborative, sustainable artifact that respects both the machine and the maintainers who will follow us.
