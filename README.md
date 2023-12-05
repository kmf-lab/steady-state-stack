# steady-state-stack
Stack of Rust crates for stead state projects

The Steady State Framework Stack (3S framework) for Rust is a focused collection of crates designed to enhance command-line interface development, concurrent processing, asynchronous programming, and logging. Here's a brief overview of its core components:

StructOpt (0.3.26): Facilitates the creation of command-line interfaces by parsing command-line arguments directly into Rust structs, ensuring type safety and ease of use.
Bastion (0.4.5): An actor framework providing a resilient, fault-tolerant environment with supervisors to recover from panics, ideal for building scalable and reliable applications.
Flume (0.11.0): Offers lock-free bounded channels for efficient and reliable communication between actors, enhancing concurrent processing without the overhead of locking mechanisms.
Log (0.4.20): Provides common logging traits, enabling consistent logging across applications and libraries.
FlexiLogger (0.27.3): A flexible logging implementation that can be tailored to various output formats and destinations, enhancing the application's logging capabilities.
Futures-Timer (3.0.2): Supports asynchronous operations like delays (using Delay::new(check_rate).await), useful in scenarios where polling is necessary.
Futures (0.3.29): A comprehensive library for asynchronous programming, offering utilities like select!, barriers, and mutexes for advanced control over asynchronous tasks.
Itertools (0.12.0): Provides additional iterator methods, enriching Rust's already powerful iteration capabilities with more sophisticated and convenient functions.
Async-Recursion (1.0.5): Simplifies the writing of asynchronous recursive functions, enabling more straightforward and readable async code.
This collection is particularly suitable for developing high-performance, concurrent, and asynchronous Rust applications, with a strong emphasis on reliability, efficient data handling, and enhanced logging capabilities.

