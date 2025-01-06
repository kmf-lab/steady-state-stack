use std::cell::RefCell;
use std::time::Duration;
use askama::Template;

//TODO: 2025 add pre-built actors with custom templates which can be adjusted later
//TODO: 2025 if generated project files already exist confirm and backup the files


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
///     Labels also contain a display name as name::DisplayName and the message struct type in angle brackets (e.g., `<Widget>`).
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

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum ConsumePattern {
    PeekCopy, //do work on peek copy for greater durability
    TakeCopy, //take using copy for faster slice processing
    Take, //take using traditional ownership handoff semantics
}


#[derive(Eq, PartialEq, Clone, Debug)]
pub(crate) struct Channel {
    pub(crate) name: String,
    pub(crate) from_mod: String,
    pub(crate) to_mod: String,
    pub(crate) bundle_struct_mod: String, //this is where the struct message_type is defined
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

    pub fn tx_prefix_distributed_name(&self) -> String {
            //format!("{}n_to_{}", self.from_mod.to_lowercase(), self.to_node.to_lowercase())
            format!("n_to_{}", self.to_node.to_lowercase())
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
            self.rebundle_index>=0
    }

    pub fn restructured_bundle(&self) -> bool {
        self.rebundle_index>=0 && !self.is_unbundled
    }
    pub fn rebundle_index(&self) -> isize { 
        self.rebundle_index
    }

    pub fn should_build_read_buffer(&self) -> bool {
        self.batch_read > 1 && self.copy
    }
    pub fn should_build_write_buffer(&self) -> bool {
        self.batch_write > 1 && self.copy
    }
}

#[derive(Default)]
pub(crate) struct Actor {
    pub(crate) display_name: String,
    pub(crate) display_suffix: Option<usize>,
    pub(crate) mod_name: String,
    pub(crate) rx_channels: Vec<Vec<Channel>>,
    pub(crate) tx_channels: Vec<Vec<Channel>>,
    pub(crate) driver: Vec<ActorDriver>,
}

impl Actor {
   pub(crate) fn is_on_graph_edge(&self) -> bool {
        self.rx_channels.is_empty() || self.tx_channels.is_empty()
   }
    
   pub(crate) fn formal_name(&self) -> String {
        if let Some(suffix) = self.display_suffix {
            format!("{}{}", self.display_name, suffix)
        } else {
            self.display_name.clone()
        }
   } 
    
}





#[derive(Template)]
#[template(path = "file_cargo.txt")]
pub(crate) struct CargoTemplate<'a> {
    pub(crate) name: &'a str,
}
#[derive(Template)]
#[template(path = "dockerfile.txt")]
pub(crate) struct DockerFileTemplate<'a> {
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
    pub(crate) note_for_the_user: String,
    pub(crate) project_name: String,
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
    pub(crate) is_on_graph_edge: bool,
    pub(crate) rx_channels: Vec<Vec<Channel>>,
    pub(crate) tx_channels: Vec<Vec<Channel>>,
    pub(crate) rx_monitor_defs: Vec<String>,
    pub(crate) tx_monitor_defs: Vec<String>,
    pub(crate) full_driver_block: String,
    pub(crate) message_types_to_use: Vec<String>,
    pub(crate) message_types_to_define: Vec<String>,

}

#[cfg(test)]
mod additional_tests {
    use super::*;
    use std::cell::RefCell;

    impl Default for Channel {
        fn default() -> Self {
            Channel {
                name: String::new(),
                from_mod: String::new(),
                to_mod: String::new(),
                bundle_struct_mod: String::new(),
                message_type: String::new(),
                peek: false,
                copy: false,
                batch_read: 1,
                to_node: String::new(),
                from_node: String::new(),
                is_unbundled: true,
                batch_write: 1,
                capacity: 0,
                bundle_index: -1,
                rebundle_index: -1,
                bundle_on_from: RefCell::new(true),
            }
        }
    }

    #[test]
    fn test_channel_needs_tx_single_clone() {
        let channel = Channel {
            is_unbundled: true,
            ..Default::default()
        };
        assert!(!channel.needs_tx_single_clone());

        let channel = Channel {
            is_unbundled: false,
            ..Default::default()
        };
        assert!(channel.needs_tx_single_clone());
    }

    #[test]
    fn test_channel_needs_rx_single_clone() {
        let channel = Channel {
            is_unbundled: true,
            ..Default::default()
        };
        assert!(!channel.needs_rx_single_clone());

        let channel = Channel {
            is_unbundled: false,
            ..Default::default()
        };
        assert!(channel.needs_rx_single_clone());
    }

    #[test]
    fn test_channel_has_bundle_index() {
        let channel = Channel {
            bundle_index: -1,
            ..Default::default()
        };
        assert!(!channel.has_bundle_index());

        let channel = Channel {
            bundle_index: 0,
            ..Default::default()
        };
        assert!(channel.has_bundle_index());
    }

    #[test]
    fn test_channel_bundle_index() {
        let channel = Channel {
            bundle_index: 5,
            ..Default::default()
        };
        assert_eq!(channel.bundle_index(), 5);
    }

    #[test]
    fn test_channel_tx_prefix_name() {
        let channel = Channel {
            from_node: "FromNode".to_string(),
            bundle_on_from: RefCell::new(true),
            ..Default::default()
        };
        let channels = vec![channel.clone()];
        assert_eq!(channel.tx_prefix_name(&channels), "fromnode");

        // let channel = Channel {
        //     to_node: "ToNode".to_string(),
        //     bundle_on_from: RefCell::new(false),
        //     ..Default::default()
        // };
        // let channels = vec![channel.clone()];
        // assert_eq!(channel.tx_prefix_name(&channels), "n_to_tonode");
    }

    #[test]
    fn test_channel_rx_prefix_name() {
        let channel = Channel {
            to_node: "ToNode".to_string(),
            bundle_on_from: RefCell::new(false),
            ..Default::default()
        };
        let channels = vec![channel.clone()];
        assert_eq!(channel.rx_prefix_name(&channels), "tonode");

        // let channel = Channel {
        //     from_node: "FromNode".to_string(),
        //     to_mod: "ToMod".to_string(),
        //     bundle_on_from: RefCell::new(true),
        //     ..Default::default()
        // };
        // let channels = vec![channel.clone()];
        // assert_eq!(channel.rx_prefix_name(&channels), "fromnode_to_tomod");
    }

    #[test]
    fn test_channel_restructured_bundle_rx() {
        let channel = Channel {
            rebundle_index: 1,
            bundle_on_from: RefCell::new(true),
            ..Default::default()
        };
        let channels = vec![channel.clone()];
        assert!(channel.restructured_bundle_rx(&channels));

        let channel = Channel {
            rebundle_index: -1,
            bundle_on_from: RefCell::new(true),
            ..Default::default()
        };
        let channels = vec![channel.clone()];
        assert!(!channel.restructured_bundle_rx(&channels));
    }

    #[test]
    fn test_channel_restructured_bundle() {
        let channel = Channel {
            rebundle_index: 1,
            is_unbundled: false,
            ..Default::default()
        };
        assert!(channel.restructured_bundle());

        let channel = Channel {
            rebundle_index: -1,
            is_unbundled: false,
            ..Default::default()
        };
        assert!(!channel.restructured_bundle());

        let channel = Channel {
            rebundle_index: 1,
            is_unbundled: true,
            ..Default::default()
        };
        assert!(!channel.restructured_bundle());
    }

    #[test]
    fn test_channel_should_build_read_buffer() {
        let channel = Channel {
            batch_read: 1,
            copy: true,
            ..Default::default()
        };
        assert!(!channel.should_build_read_buffer());

        let channel = Channel {
            batch_read: 2,
            copy: true,
            ..Default::default()
        };
        assert!(channel.should_build_read_buffer());

        let channel = Channel {
            batch_read: 2,
            copy: false,
            ..Default::default()
        };
        assert!(!channel.should_build_read_buffer());
    }

    #[test]
    fn test_channel_should_build_write_buffer() {
        let channel = Channel {
            batch_write: 1,
            copy: true,
            ..Default::default()
        };
        assert!(!channel.should_build_write_buffer());

        let channel = Channel {
            batch_write: 2,
            copy: true,
            ..Default::default()
        };
        assert!(channel.should_build_write_buffer());

        let channel = Channel {
            batch_write: 2,
            copy: false,
            ..Default::default()
        };
        assert!(!channel.should_build_write_buffer());
    }

    #[test]
    fn test_actor_is_on_graph_edge() {
        let actor = Actor {
            rx_channels: vec![],
            tx_channels: vec![vec![]],
            ..Default::default()
        };
        assert!(actor.is_on_graph_edge());

        let actor = Actor {
            rx_channels: vec![vec![]],
            tx_channels: vec![],
            ..Default::default()
        };
        assert!(actor.is_on_graph_edge());

        let actor = Actor {
            rx_channels: vec![vec![]],
            tx_channels: vec![vec![]],
            ..Default::default()
        };
        assert!(!actor.is_on_graph_edge());
    }

    #[test]
    fn test_actor_formal_name() {
        let actor = Actor {
            display_name: "ActorName".to_string(),
            display_suffix: Some(1),
            ..Default::default()
        };
        assert_eq!(actor.formal_name(), "ActorName1");

        let actor = Actor {
            display_name: "ActorName".to_string(),
            display_suffix: None,
            ..Default::default()
        };
        assert_eq!(actor.formal_name(), "ActorName");
    }
}

