## Steady State Framework: A Technical Overview

**Steady State** is a high-performance, actor-based concurrency framework in Rust designed to simplify building resilient, scalable, and observable systems. It’s ideal for developers ranging from beginners to technical leads, offering a balance of ease of use and cutting-edge performance. Whether you’re prototyping a startup idea, crafting a maker project, or architecting a large-scale business solution, Steady State provides the tools to handle massive workloads with confidence.

### Why Choose Steady State?

Steady State leverages Rust’s safety guarantees and the actor model to deliver a framework that’s both powerful and approachable. Here’s what sets it apart:

- **Safe Concurrency**: Actors operate as isolated units with no shared memory, communicating via message passing to eliminate race conditions and deadlocks.
- **High Throughput**: Capable of processing **hundreds of millions of 64-bit messages per second**, saturating network or SSD bandwidth on modern hardware.
- **Rapid Development**: Intuitive APIs and comprehensive testing tools accelerate prototyping and iteration.
- **Observability**: Built-in telemetry and Prometheus integration provide real-time insights into system health and performance.

### Key Technical Features

#### Actor Model Foundations
- **Isolated Actors**: Each actor manages its own state and communicates via channels, ensuring predictable, crash-free execution.
- **Flexible Management**: Choose dedicated threads (`SoloAct`) for isolation or shared threads (`MemberOf`) for lightweight coordination.

#### Performance Engineering
- **Massive Throughput**: Benchmarks show **50–300 million messages per second** with optimized batching and large buffers (e.g., 1,048,576-message channels).
- **Double-Buffer Batching**: Actors process half a channel’s capacity (e.g., 524,288 messages) while the other half fills, maximizing CPU cache efficiency.
- **Zero-Copy Processing**: Advanced users can operate directly on channel memory with `peek_slice`/`poke_slice`, eliminating allocations for peak performance.

#### Resilience and Robustness
- **Persistent State**: `SteadyState<T>` preserves actor state across restarts, ensuring no data loss after failures.
- **Automatic Recovery**: Actors restart seamlessly on panic, with peek-before-commit ensuring message integrity.
- **Showstopper Detection**: Identifies and handles problematic messages to prevent infinite crash loops.

#### Observability and Testing
- **Telemetry Dashboard**: Access real-time metrics at `http://127.0.0.1:9900`—CPU usage, throughput, channel fill, and more.
- **Prometheus Metrics**: Scrape detailed stats at `/metrics` for integration with monitoring tools like Grafana.
- **Testing Framework**: Unit test individual actors or simulate entire graphs with tools like `GraphBuilder::for_testing()`.

#### Distributed Capabilities
- **Aeron Integration**: Use Aeron’s IPC or UDP for high-speed, reliable messaging across processes or machines.
- **Scalable Design**: Split workloads into pods for pub-sub scalability.

### Learning Through Lessons

Rather than relying on a soon-to-be-replaced code generator (currently under development), Steady State provides a series of hands-on lessons to help you build projects from scratch. These lessons, available at [https://github.com/kmf-lab](https://github.com/kmf-lab), showcase real-world patterns and are the recommended starting point:

- **Lesson 00: Minimal**  
  A single-actor system with basic timing and shutdown coordination. Perfect for understanding the actor model basics.

- **Lesson 01: Standard**  
  A multi-actor pipeline with persistent state, batch processing, and telemetry. Ideal for production-grade systems.

- **Lesson 02A: Robust**  
  Fault-tolerant design with automatic restarts, state recovery, and peek-before-commit. Essential for resilient applications.

- **Lesson 02B: Performant**  
  High-throughput optimizations with large channels, double-buffering, and zero-copy processing. Targets enterprise-scale performance.

Each lesson builds on the previous, offering source code, detailed explanations, and telemetry examples. Start here to master Steady State’s capabilities and tailor solutions to your needs.

### Getting Started

1. **Clone the Lessons**
   ```bash
   git clone https://github.com/kmf-lab/<lesson>.git
   cd <lesson>
   ```

2. **Run a Lesson**  
   Example for Lesson 02B (Performant):
   ```bash
   cd lessons/lesson-02B-performant
   cargo run -- --rate 2 --beats 30000
   ```
   Observe throughput exceeding **100M messages/sec** via `http://127.0.0.1:9900`.

3. **Explore Telemetry**
  - Dashboard: `http://127.0.0.1:9900`
  - Metrics: `http://127.0.0.1:9900/metrics`
  - Graph: `http://127.0.0.1:9900/graph.dot`

4. **Build Your Project**  
   Use the lessons as templates, leveraging Steady State’s APIs to define actor graphs, implement logic, and test comprehensively.

### Technical Highlights from Lessons

- **Lesson 02B Example**: Achieves **150–300M messages/sec** with zero-copy mode, using:
  ```rust
  let (peek_a, peek_b) = actor.peek_slice(&mut generator_rx);
  let (poke_a, poke_b) = actor.poke_slice(&mut logger_tx);
  // In-place transformation
  for i in 0..peek_a.len() { poke_a[i].write(FizzBuzzMessage::new(peek_a[i])); }
  ```
- **Lesson 02A Example**: Ensures message integrity with peek-before-commit:
  ```rust
  if let Some(&value) = actor.try_peek(&mut generator) {
      // Process, then commit
      actor.try_take(&mut generator).expect("Processed successfully");
  }
  ```

### Future Directions

- **Enhanced Distributed**: Scale up workers, Adding MQTT as well as more Aeron.
- **Visualization Boosts**: Adding new 3D actor visualization across multiple machines on one screen.
- **Project Code Generation**: Define your app in simple Toml and let the generator mock out the project.
- **Community Contributions**: Add your own actors and features via [GitHub](https://github.com/kmf-lab).

### Join the Community

Steady State is MIT-licensed and open for collaboration. Visit [https://github.com/kmf-lab](https://github.com/kmf-lab) to:
- Explore the lessons
- Report issues
- Contribute code
- Sponsor development
