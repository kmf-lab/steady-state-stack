use std::time::Duration;
use askama::Template;

//TODO: add custom actor templates which can be adjusted later?
//TODO: need to add a way to specify the actor/channel behavior on dot
//      set default behavior for now
//TODO: if files exist confirm and backup the files?


/// DOT File Notation Guide for Rust Project Actor and Channel Configurations
///
/// This guide outlines the conventions adopted for documenting actors and channels within DOT files,
/// focusing on clarity, intuition, and ease of parsing for code generation and documentation purposes.
///
/// Actor Notation:
/// - Define actors with a name.
/// - Document the actor's module, consume pattern, driver patterns, and redundancy using a label.
///   - Module: Use `mod::ModuleName` to specify the Rust module name.
///   - Consume Pattern: Indicate how the actor consumes data with `>>` followed by the pattern (e.g., `TakeCopy`).
///   - Driver Patterns: Specify activation triggers using `->` and combine conditions with `&` (AND) and `|` (OR).
///   - Redundancy: Denote multiple instances of an actor with `*` followed by the count (omit if only one instance).
///
/// Channel Notation:
/// - Define channels by their source and target node names
/// - Use labels to specify channel capacity with `#` followed by the number and optionally the bundle size with `*`.
///   Labels also contain a display name as name::DisplayName and the message struct type in angle brackets (e.g., `<Widget>`).
///
/// Examples:
/// ```
/// "A1" [label="mod::mod_a, >>TakeCopy, Every(5000ms) && OnEvent(C1//10||B2//10) && OnCapacity(C2//20||A1//20) && Other(server2,socet1), *2"];
/// "A1" -> "B2" [label="name::goes <String> #1024, *3"]; // Channel with capacity of 1024 and bundled in groups of 3
/// "C2" -> "A1" [label="name::feedback <Data> #512"]; // Channel with capacity of 512, no bundling indicated
/// ```
///
/// Relationships:
/// - Document the relationship from actors to channels focusing on the consume pattern.
/// - Use the `->` symbol to indicate the direction from actor to channel.
///
/// This notation aims to balance between the expressiveness needed for Rust project documentation and the clarity required for quick understanding and effective parsing by tools and developers alike.


// Actors are async and can be replaced at any moment if they panic.
// Data on channels however is never lost.  To Implement this it is
// required that channel locks are held to Tx and Rx messages.
// As a result there is some locking overhead which is overcome
// by the use of batching so the overhead per message is reduced.

#[derive(Debug, PartialEq)]
pub(crate) enum ActorDriver {
    Periodic(Duration),
    EventDriven(Vec<(String,usize)>), //source_node, available batch required
    CapacityDriven(Vec<(String,usize)>), //target_node, vacant batch required
    Other(Vec<String>),
}

#[derive(Debug, PartialEq)]
pub(crate) enum ConsumePattern {
    PeekCopy, //do work on peek copy for greater durability
    TakeCopy, //take using copy for faster slice processing
    Take, //take using traditional ownership semantics
}
pub(crate) struct ActorBehaviorStrategy {
    pub(crate) driver: Vec<ActorDriver>,
    pub(crate) consume: ConsumePattern,
}

#[derive(Eq, PartialEq, Hash, Clone)]
pub(crate) struct Channel {
    pub(crate) name: String,
    pub(crate) from_mod: String, //this is where the struct message_type is defined
    pub(crate) message_type: String,
    pub(crate) capacity: usize,
    // >1 is a bundle, which means this represents <Gurth> channel defs
    // these can be consolidated into a single vec on Tx, Rx or both ends
    // this does not change behavior just provides and easier way to write it
    pub(crate) gurth: usize,  // <=1 is not a bundle
}
pub(crate) struct Actor {
    pub(crate) display_name: String,
    pub(crate) mod_name: String,
    pub(crate) rx_channels: Vec<Channel>,
    pub(crate) tx_channels: Vec<Channel>,
    pub(crate) behavior: ActorBehaviorStrategy,
}


#[derive(Template)]
#[template(path = "file_cargo.txt")]
pub(crate) struct CargoTemplate<'a> {
    pub(crate) name: &'a str,
}
#[derive(Template)]
#[template(path = "file_gitignore.txt")]
pub(crate) struct GitIgnoreTemplate {
}

#[derive(Template)]
#[template(path = "file_args.txt")]
pub(crate) struct ArgsTemplate {
}





#[derive(Template)]
#[template(path = "file_main.txt")]
pub(crate) struct MainTemplate<'a> {
    pub(crate) actors: &'a Vec<Actor>,
    pub(crate) channels: &'a Vec<Channel>,
}

#[derive(Template)]
#[template(path = "file_actor.txt")]
pub(crate) struct ActorTemplate {
    pub(crate) display_name: String,
    pub(crate) rx_channels: Vec<Channel>,
    pub(crate) tx_channels: Vec<Channel>,
    //pub(crate) message_types_to_use: Vec<String>, //full namespace
    //pub(crate) message_types_to_define: Vec<String>, //struct name, with copy trate?

}
