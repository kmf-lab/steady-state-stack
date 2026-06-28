/// Defines configuration options for the proactor, which manages I/O operations.
///
/// This enum specifies different strategies for handling I/O, each tailored to specific performance needs.
#[derive(Clone, Debug)]
// ss[related graph.for-testing]
pub enum ProactorConfig {
    /// Configures the proactor for interrupt-driven I/O with minimal CPU usage.
    ///
    /// This option is suitable for low-throughput scenarios where completion latency is less critical.
    InterruptDriven,
    /// Configures the proactor to use kernel polling for efficient I/O in high-traffic environments.
    ///
    /// This balances throughput and resource usage without aggressive polling.
    KernelPollDriven,
    /// Configures the proactor for low-latency, high-throughput I/O operations.
    ///
    /// This option prioritizes performance, consuming more resources for demanding workloads.
    LowLatencyDriven,
    /// Configures the proactor with I/O polling for low-latency file operations.
    ///
    /// This is optimized for file-based I/O, reducing latency in such contexts.
    IoPoll,
}
