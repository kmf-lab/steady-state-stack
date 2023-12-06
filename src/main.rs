

// Re-export entire crates
pub use structopt::*; //TODO: show args
pub use bastion; //TODO: show supers.
pub use flume;  //TODO: show try and log of full pipe
pub use log;   //TODO: add some logging
pub use flexi_logger;
pub use futures_timer; //TODO: show delay on time..
pub use futures; // TODO: show a Select! ??
pub use itertools; // TODO: show a cool iterator
pub use async_recursion; // TODO: show a recursion


mod actor {
    pub mod example_empty_actor;
    pub mod data_generator;
    pub mod data_approval;
    pub mod data_consumer;
}


fn main() {
    println!("Hello, world!");


}

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
