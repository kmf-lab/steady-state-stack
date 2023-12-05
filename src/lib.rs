// Re-export entire crates
pub use structopt::*;
pub use bastion;
pub use flume;
pub use log;
pub use flexi_logger;
pub use futures_timer;
pub use futures;
pub use itertools;
pub use async_recursion;


#[cfg(feature = "test-utils")]
pub use async_std;


// Example: StructOpt usage example
// Requires the StructOpt derive macro and its traits to be available
#[cfg(feature = "cli")]
pub use structopt::StructOpt;

#[cfg(feature = "cli")]
#[derive(Debug, structopt::StructOpt)]
pub struct CliArgs {
    /// Example argument
    pub arg: String,
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "test-utils")]
    use async_std;

    #[cfg(feature = "test-utils")]
    #[test]
    async fn my_async_test() {
        // Your test code that uses async-std here
    }
}

