# steady-state-stack
Stack of Rust crates for steady state projects


# new projects can use SSS as a starting point with this.
cargo new --name my_new_project --template https://github.com/kmf-lab/steady-state-stack.git

This will be a crate once all the key features are in place.

Design Features:

Safety
Borrow Checker: Ensures memory safety without garbage collection, preventing common errors like null pointer dereferences and buffer overruns.
Actor-Based Threading: Reduces complexity and potential for race conditions by encapsulating state within actors, leading to safer concurrency management.

Reliability
Erlang Style Supervisors: Implements a supervision strategy for fault tolerance, ensuring system resilience and continuous operation.
Dynamic Thread Scaling: Automatically scales thread usage based on workload, ensuring efficient resource utilization without overloading.

Latency
Bounded Monitored Channels: Enables precise control over system latency, ensuring timely data processing and transfer.
Lock-Free Asynchronous Message Passing: Improves system responsiveness by reducing blocking and waiting times during communication.

Performance
Efficient Async Operations: Maximizes CPU usage with non-blocking code, leading to faster execution and better resource management.
Optimized Thread Management: Balances workload across threads for optimal performance, avoiding both underutilization and bottlenecks.

Maintenance
Comprehensive Testing Strategy: Includes both unit and application-level testing to ensure robustness and minimize bugs.
Actor Isolation for Debugging: Facilitates easier troubleshooting and maintenance by isolating functionality within individual actors.

Throughput
High Message Throughput: Efficiently handles a large number of messages per second, crucial for high-load systems.
Steady State Processing: Maintains consistent processing capacity, ensuring reliable throughput even under varying loads.

Scalability
Horizontal and Vertical Scaling: Adapts to increased demands by scaling out (adding more nodes) or up (adding resources to existing nodes).
Load Balancing: Distributes workload evenly across available resources, preventing any single point of overutilization.

Flexibility
Modular Design: Allows for easy swapping and updating of components, supporting a wide range of use cases and evolving requirements.
Cross-Platform Compatibility: Functions seamlessly across different operating systems and hardware, enhancing its applicability.

Security
Rigorous Error Handling: Prevents cascading failures and vulnerabilities by handling errors gracefully.
Secure Communication Channels: Ensures data integrity and confidentiality during actor communications.
Strong type constraints on command line args.



The Steady State Stack (3S framework) for Rust is a focused collection of crates
designed to enhance general service development, concurrent processing, 
asynchronous programming, and logging. Here's a brief overview of its core components:

StructOpt (0.3.26): Facilitates the creation of command-line interfaces by parsing command-line arguments directly into Rust structs, ensuring type safety and ease of use.

Bastion (0.4.5): An actor framework providing a resilient, fault-tolerant environment with supervisors to recover from panics, ideal for building scalable and reliable applications.

async-ringbuf (0.2.0-rc.4): Offers lock-free bounded channels for efficient and reliable communication between actors, enhancing concurrent processing without the overhead of locking mechanisms.

Log (0.4.20): Provides common logging traits, enabling consistent logging across applications and libraries.

FlexiLogger (0.27.3): A flexible logging implementation that can be tailored to various output formats and destinations, enhancing the application's logging capabilities.

Futures-Timer (3.0.2): Supports asynchronous operations like delays (using Delay::new(check_rate).await), useful in scenarios where polling is necessary.

Futures (0.3.29): A comprehensive library for asynchronous programming, offering utilities like select!, barriers, and mutexes for advanced control over asynchronous tasks.

Itertools (0.12.0): Provides additional iterator methods, enriching Rust's already powerful iteration capabilities with more sophisticated and convenient functions.

Async-Recursion (1.0.5): Simplifies the writing of asynchronous recursive functions, enabling more straightforward and readable async code.

This collection is particularly suitable for developing high-performance, concurrent, and asynchronous Rust applications, with a strong emphasis on reliability, efficient data handling, and enhanced logging capabilities.

