use std::cell::RefCell;
use std::time::Duration;
use askama::Template;

//TODO: add custom actor templates which can be adjusted later?
//TODO: if files exist confirm and backup the files?


/// DOT File Notation Guide for Rust Project Actor and Channel Configurations
///
/// This guide outlines the conventions adopted for documenting actors and channels within DOT files,
/// focusing on clarity, intuition, and ease of parsing for code generation and documentation purposes.
///
/// Actor Notation:
/// - Define actors with a name.
/// - Document the actor's module, consume pattern, and driver patterns using a label.
///   - Module: Use `mod::ModuleName` to specify the Rust module name.
///   - Driver Patterns: Specify activation triggers using AtLeastEvery, OnEvent etc and combine conditions with `&&`
///      - AtLeastEvery: Denote a minimum time interval between actor activations.
///      - AtMostEvery: Denote a maximum time interval between actor activations.
///      - OnEvent: channelName:batchSize(optional, default is 1):girthApplication(Optional, is all or any or x%)
///      - OnCapacity: Denote an actor that activates when a specific target node has capacity.
///      - Other: Denote any other activation conditions.
///
/// Channel Notation:
/// - Define channels by their source and target node names
/// - Use labels to specify channel capacity with `#` followed by the number and optionally the bundle flag with `*B`.
///   - Consume Pattern: Indicate how the actor consumes data with `>>` followed by the pattern (e.g., `TakeCopy`).
///   Labels also contain a display name as name::DisplayName and the message struct type in angle brackets (e.g., `<Widget>`).
///
/// Examples:
/// ```
/// "A1" [label="mod::mod_a, AtLeastEvery(5000ms) && OnEvent(goes:10:32%||feedback:10) && OnCapacity(feedback:20||goes:20) && Other(server2||socet1), *2"];
/// "A1" -> "B2" [label="name::goes >>TakeCopy, <String> #1024, *B"]; // Channel with capacity of 1024 and bundled in groups of 3
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
    AtLeastEvery(Duration),
    AtMostEvery(Duration),
    EventDriven(Vec<Vec<String>>), //source_node, available batch required
    CapacityDriven(Vec<Vec<String>>), //target_node, vacant batch required
    Other(Vec<String>),
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) enum ConsumePattern {
    PeekCopy, //do work on peek copy for greater durability
    TakeCopy, //take using copy for faster slice processing
    Take, //take using traditional ownership handoff semantics
}


#[derive(Eq, PartialEq, Clone)]
pub(crate) struct Channel {
    pub(crate) name: String,
    pub(crate) from_mod: String, //this is where the struct message_type is defined
    pub(crate) to_mod: String,
    pub(crate) message_type: String,
    pub(crate) peek: bool,
    pub(crate) copy: bool,
    pub(crate) batch_read: usize,

    pub(crate) to_node: String,
    pub(crate) from_node: String,
    pub(crate) is_unbundled: bool,
    pub(crate) batch_write: usize,
    pub(crate) capacity: usize,
    pub(crate) bundle_index: isize,
    pub(crate) rebundle_index: isize,
    pub(crate) bundle_on_from: RefCell<bool>,
}

impl Channel {

    pub fn needs_tx_single_clone(&self) -> bool {
                !self.is_unbundled
    }
    pub fn needs_rx_single_clone(&self) -> bool {
                !self.is_unbundled
    }

    pub fn has_bundle_index(&self) -> bool {
                 self.bundle_index>=0
    }
    pub fn bundle_index(&self) -> isize {
                 self.bundle_index
    }

    // if we fan out we use the mod name so the [index] is clear
    pub fn tx_prefix_name(&self, channels:&[Channel]) -> String {
        if *self.bundle_on_from.borrow() || channels.len()<=1 {
            self.from_node.to_lowercase()
        } else {
            self.tx_prefix_distributed_name()
        }
    }

    /// Used for building the synthetic bundles
    pub fn tx_prefix_distributed_name(&self) -> String {
            format!("{}n_to_{}", self.from_mod.to_lowercase(), self.to_node.to_lowercase())

    }

    // if we fan out we use the mod name so the [index] is clear
    pub fn rx_prefix_name(&self, channels:&[Channel]) -> String {
        if !*self.bundle_on_from.borrow() || channels.len()<=1 {
            self.to_node.to_lowercase()
        } else {
            self.rx_prefix_distributed_name()
        }
    }
    pub fn rx_prefix_distributed_name(&self) -> String {
        format!("{}_to_{}",self.from_node.to_lowercase(), self.to_mod.to_lowercase())
    }

    pub fn restructured_bundle_rx(&self, _channels:&[Channel]) -> bool {
        //special case where we do not want this def because we already have it
        *self.bundle_on_from.borrow() &&
            -1==self.bundle_index && self.rebundle_index>=0
    }

    pub fn restructured_bundle(&self) -> bool { -1==self.bundle_index && self.rebundle_index>=0 }
    pub fn rebundle_index(&self) -> isize { self.rebundle_index }


    pub fn should_build_read_buffer(&self) -> bool {
        self.batch_read > 1 && self.copy
    }
    pub fn should_build_write_buffer(&self) -> bool {
        self.batch_write > 1 && self.copy
    }
}


pub(crate) struct Actor {
    pub(crate) display_name: String,
    pub(crate) mod_name: String,
    pub(crate) rx_channels: Vec<Vec<Channel>>,
    pub(crate) tx_channels: Vec<Vec<Channel>>,
    pub(crate) driver: Vec<ActorDriver>,
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
    pub(crate) test_only: &'static str,
    pub(crate) actors: &'a Vec<Actor>,
    pub(crate) actor_mods: Vec<String>,
    pub(crate) channels: &'a Vec<Vec<Channel>>,
}

#[derive(Template)]
#[template(path = "file_actor.txt")]
pub(crate) struct ActorTemplate {
    pub(crate) note_for_the_user: String,
    pub(crate) display_name: String,
    pub(crate) has_bundles: bool,
    pub(crate) rx_channels: Vec<Vec<Channel>>,
    pub(crate) tx_channels: Vec<Vec<Channel>>,
    pub(crate) rx_monitor_defs: Vec<String>,
    pub(crate) tx_monitor_defs: Vec<String>,
    pub(crate) full_driver_block: String,
    pub(crate) full_process_example_block: String,
    pub(crate) message_types_to_use: Vec<String>,
    pub(crate) message_types_to_define: Vec<String>,

}
