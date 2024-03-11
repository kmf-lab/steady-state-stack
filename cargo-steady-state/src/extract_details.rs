use dot_parser::canonical::Graph;
use std::error::Error;
use std::time::Duration;
use log::error;
use crate::ProjectModel;
use crate::templates::{Actor, ActorDriver, Channel, ConsumePattern};

fn extract_type_name_from_edge_label(label_text: &str, from_node: &str, to_node: &str) -> String {
    ///////////////////////////////////
    // Attempt to find the type name
    // This is preferred if it exists
    //////////////////////////////////
    if let (Some(start), Some(end)) = (label_text.find('<'), label_text.find('>')) {
        if start < end {
            let type_name = &label_text[start + 1..end];
            // Check if the extracted part has no whitespace and starts with a capital letter
            if type_name.chars()
                        .all(|ch| !ch.is_whitespace())      {
                if let Some(c) = type_name.chars().next() {
                    if c.is_uppercase() {
                        return type_name.to_string();
                    }
                }
            }
        }
    }

    ////////////////////////////////////////////////////////////////////////////////
    //after this point we will attempt to select something helpful from the label
    ////////////////////////////////////////////////////////////////////////////////
    // If the first attempt fails, split by newline and process the first line
    let input = label_text.replace("\\n","\n").replace('"',"");
    let first_line = input.lines().next();
    if let Some(first_line) = first_line {

        let parts: Vec<String> = first_line.split_whitespace()
            .map(|s| {
                let mut chars = s.chars();
                match chars.next() {
                    Some(first_char) =>
                        {
                            if first_char.is_alphabetic() {
                                first_char.to_uppercase().collect::<String>() + chars.as_str()
                            } else {
                                format!("From{}To{}",from_node,to_node).to_string()
                            }
                        }
                    ,
                    None => s.to_string(),
                }
            })
            .collect();
        if let Some(x) = parts.first() {
            if !x.is_empty() {
                let joined = parts.join("");
                return joined;
            }
        }
    }
    //if there is no label then we build up a type based on from and to nodes
    format!("From{}To{}",from_node,to_node).to_string()
}

fn extract_capacity_from_edge_label(label_text: &str, default: usize) -> usize {
    if let Some(start) = label_text.find('#') {
        let remaining = &label_text[start + 1..];
        if let Some(end) = remaining.find(|c: char| !c.is_ascii_digit()) {
            remaining[..end].parse::<usize>().unwrap_or(default)
        } else {
            remaining.parse::<usize>().unwrap_or(default)
        }
    } else {
        default // Default capacity when not specified
    }
}


fn extract_gurth_from_edge_label(label_text: &str) -> bool {
    label_text.contains("*B")
}

fn extract_redundancy_count(label_text: &str) -> usize {
    if let Some(start) = label_text.find('*') {
        let remaining = &label_text[start + 1..];
        if let Some(end) = remaining.find(|c: char| !c.is_ascii_digit()) {
            remaining[..end].parse::<usize>().unwrap_or(1)
        } else {
            remaining.parse::<usize>().unwrap_or(1)
        }
    } else {
        1 // Default redundancy when not specified
    }
}

fn extract_module_name(node_id: &str, label_text: &str) -> String {
    let module_prefix = "mod::";
    if let Some(start) = label_text.find(module_prefix) {
        let remaining = &label_text[start + module_prefix.len()..];
        // Find the first occurrence of either a comma or a whitespace character
        if let Some(end) = remaining.find(|c: char| c == ',' || c.is_whitespace()) {
            remaining[..end].to_string()
        } else {
            remaining.to_string()
        }
    } else {
        //convert nodeId from camel case to snake case

        let result = to_snake_case(node_id);
        // Use the node_id to form a default module name
        format!("mod_{}", result)
    }
}

fn to_snake_case(input: &str) -> String {
    let mut result = String::new();
    for (i, c) in input.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        if let Some(lower) = c.to_lowercase().next() {
            result.push(lower);
        }
    }
    result
}

fn extract_consume_pattern_from_label(label: &str) -> ConsumePattern {
    if label.contains(">>PeekCopy") {
        ConsumePattern::PeekCopy
    } else if label.contains(">>TakeCopy") {
        ConsumePattern::TakeCopy
    } else { // could be ">>Take" or unknown default
        ConsumePattern::Take
    }
}


fn find_start_position(label: &str) -> usize {
    let keywords = ["AtMostEvery(", "AtLeastEvery(", "OnEvent(", "OnCapacity(", "Other("];
    keywords.iter()
        .filter_map(|&keyword| label.find(keyword))
        .min() // Find the earliest occurrence of any keyword
        .unwrap_or(label.len()) // Default
}

fn extract_actor_driver_from_label(label: &str) -> Vec<ActorDriver> {

    let start_pos = find_start_position(label);

    let mut result: Vec<ActorDriver> =
    label[start_pos..].split("&&").filter_map(|part| {
        let part = part.trim();
        if part.starts_with("AtMostEvery") { //Timeout, Countdown. AtLestEvery, AtMostEvery
            part.strip_prefix("AtMostEvery(")
                .and_then(|s| s.strip_suffix("ms)"))
                .and_then(|ms| ms.trim().parse::<u64>().ok())
                .map(Duration::from_millis)
                .map(ActorDriver::AtMostEvery)
        } else if part.starts_with("AtLeastEvery") { //Timeout, Countdown. AtLestEvery, AtMostEvery
                part.strip_prefix("AtLeastEvery(")
                    .and_then(|s| s.strip_suffix("ms)"))
                    .and_then(|ms| ms.trim().parse::<u64>().ok())
                    .map(Duration::from_millis)
                    .map(ActorDriver::AtLeastEvery)
        } else if part.starts_with("OnEvent") {
            Some(ActorDriver::EventDriven(parse_pairs(part, "OnEvent")))
        } else if part.starts_with("OnCapacity") {
            Some(ActorDriver::CapacityDriven(parse_pairs(part, "OnCapacity")))
        } else if part.starts_with("Other") {
            part.strip_prefix("Other(")
                .and_then(|s| s.strip_suffix(')'))
                .map(|items| items.split(',').map(|item| item.trim().to_string()).collect())
                .map(ActorDriver::Other)
        } else {
            None
        }
    }).collect();
    if result.is_empty() {
        // Default driver, probably not right; but we have little choice
        result.push(ActorDriver::AtMostEvery(Duration::from_secs(1)));
    }
    result
}

fn parse_pairs(part: &str, prefix: &str) -> Vec<(String, usize)> {
    part.strip_prefix(prefix)
        .and_then(|s| s.split_once('('))
        .map(|(_, rest)| rest.trim().strip_suffix(')').unwrap_or(rest))
        .unwrap_or("")
        .split("||")
        .filter_map(|pair| {
            let (node, batch) = pair.split_once("//")?;
            Some((node.trim().to_string(), batch.trim().parse::<usize>().ok()?))
        })
        .collect()
}

fn extract_channel_name(label_text: &str, from_node: &str, to_node: &str) -> String {
    let module_prefix = "name::";
    if let Some(start) = label_text.find(module_prefix) {
        let remaining = &label_text[start + module_prefix.len()..];
        // Find the first occurrence of either a comma or a whitespace character
        if let Some(end) = remaining.find(|c: char| c == ',' || c.is_whitespace()) {
            remaining[..end].to_string()
        } else {
            remaining.to_string()
        }
    } else {
        // Use the node_id to form a default module name
        format!("{}_to_{}", to_snake_case(from_node), to_snake_case(to_node))
    }
}



/////////////////////////
////////////////////////
pub(crate) fn extract_project_model<'a>(name: &str, g: Graph<'a, (&'a str, &'a str)>) -> Result<ProjectModel, Box<dyn Error>> {
    let mut pm = ProjectModel { name: name.to_string(), ..Default::default() };

    // Iterate over nodes to populate actors
    for node in &g.nodes.set {
        let id = node.1.id;
        let label_text = node.1.attr.elems
                          .iter()
                          .find_map(|(key, value)| if "label".eq(*key) { Some(*value) } else { None }).unwrap_or_default();

        let mod_name = extract_module_name(id, label_text);

        let redundancy_count = extract_redundancy_count(label_text);

        // Create an Actor instance based on extracted details
        let actor = Actor {
            display_name: id.to_string(),  // Assuming the display_name is the node id
            mod_name,
            rx_channels: Vec::new(),  // Populated later based on edges
            tx_channels: Vec::new(),  // Populated later based on edges
            driver: extract_actor_driver_from_label(label_text),
        };
        pm.actors.push(actor);
    }

    // Iterate over edges to populate channels
    for e in &g.edges.set {
        let label_text = e.attr.elems
            .iter()
            .find_map(|(key, value)| if "label".eq(*key) { Some(*value) } else { None }).unwrap_or_default();

        let type_name = extract_type_name_from_edge_label(label_text, e.from, e.to);
        let capacity = extract_capacity_from_edge_label(label_text, 8);  // Assuming 8 as default if not specified

        let is_part_of_bundle = extract_gurth_from_edge_label(label_text);

        let name = extract_channel_name(label_text, e.from, e.to);
        let consume_pattern = extract_consume_pattern_from_label(label_text);

        if let Some(mod_name) = pm.actors.iter().filter(|f| f.display_name == e.from).map(|a| a.mod_name.clone()).next() {

            // Create a Channel instance based on extracted details
            let mut channel = Channel {
                name,
                from_mod: mod_name,
                batch_read: 1,  //default replaced later if set
                batch_write: 1,  //default replaced later if set
                message_type: type_name,
                peek: consume_pattern == ConsumePattern::PeekCopy,
                copy: consume_pattern != ConsumePattern::Take,
                capacity,
                part_of_bundle: is_part_of_bundle,
            };

            // Find the actor with the same id as the from node and add the channel to its tx_channels
            if let Some(a) = pm.actors.iter_mut().find(|f| f.display_name == e.from) {
                  //we found the actor which is the source of this channel

                  //review this actor and see if it has some batched write size
                  a.driver.iter().for_each(|f| {
                        if let ActorDriver::CapacityDriven(pairs) = f {
                            pairs.iter()
                                .filter(|(n, _)| n.eq(&channel.name))
                                .for_each(|(_, b)| channel.batch_write = *b );
                        };
                  });
                  roll_up_bundle(&mut a.tx_channels, channel.clone());
            }
            if let Some(a) = pm.actors.iter_mut().find(|f| f.display_name == e.to) {
                  //we found the actor which is the source of this channel

                  //review this actor and see if it has some batched write size
                  a.driver.iter().for_each(|f| {
                        if let ActorDriver::EventDriven(pairs) = f {
                            pairs.iter()
                                .filter(|(n, _)| n.eq(&channel.name))
                                .for_each(|(_, b)| channel.batch_read = *b );
                        };
                  });
                roll_up_bundle(&mut a.rx_channels, channel.clone());
            }
            roll_up_bundle(&mut pm.channels, channel);
        } else {
            error!("Failed to find actor with id: {}", e.from);
        }


    }




    Ok(pm)
}


/// This function is used to roll up channels into bundles and is important for the code generation
/// Some Channels are grouped into vecs because they are all the same and either originate
/// or terminate at the same actor. This simplifies code to allow for indexing of channels.
fn roll_up_bundle(collection: &mut Vec<Vec<Channel>>, insert_me: Channel) {

    if insert_me.part_of_bundle {
        if let Some(x) = collection.iter_mut().find(|f|
                 f[0].part_of_bundle
                 && f[0].name.eq(&insert_me.name) // we all agree on the name
                 && f[0].capacity.eq(&insert_me.capacity) // we all agree on the capacity
                 && f[0].from_mod.eq(&insert_me.from_mod) // we all agree where our type is defined
                 && f[0].message_type.eq(&insert_me.message_type) // we all agree on the type
                 && f[0].copy.eq(&insert_me.copy) // we all agree on the copy semantics
        ) { //this is clearly part of the same bundle
            x.push(insert_me);
        } else {
            collection.push(vec![insert_me]);
        }
    } else {
        collection.push(vec![insert_me]);
    }



}
////////
///////


#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use crate::extract_details;
    use crate::extract_details::*;

    #[test]
    fn test_extract_type_name_from_edge_label() {
        let label = "IMAP server details\nemail, password";
        let from = "ConfigLoader";
        let to = "IMAPClient";
        let result = extract_details::extract_type_name_from_edge_label(label, from, to);
        assert_eq!(result, "IMAPServerDetails".to_string());
    }

    #[test]
    fn test_extract_type_name_from_edge_label2() {
        let label = "<Widget>#1024";
        assert_eq!(extract_type_name_from_edge_label(label, "NodeA", "NodeB"), "Widget".to_string());

        let label_with_junk = "Some text <WidgetType>#512 more text";
        assert_eq!(extract_type_name_from_edge_label(label_with_junk, "NodeA", "NodeB"), "WidgetType".to_string());

        // Test with missing type name
        let label_missing_type = "#1024";
        assert_eq!(extract_type_name_from_edge_label(label_missing_type, "NodeA", "NodeB"), "FromNodeAToNodeB".to_string());
    }

    #[test]
    fn test_extract_capacity_from_edge_label() {
        let label = "Capacity #1024";
        assert_eq!(extract_capacity_from_edge_label(label, 512), 1024);

        // Test default capacity
        let label_missing_capacity = "No capacity here";
        assert_eq!(extract_capacity_from_edge_label(label_missing_capacity, 512), 512);
    }

    #[test]
    fn test_extract_gurth_from_edge_label() {
        let label = "Gurth *B";
        assert_eq!(extract_gurth_from_edge_label(label),true);

        // Test default gurth
        let label_missing_gurth = "No gurth specified";
        assert_eq!(extract_gurth_from_edge_label(label_missing_gurth),false);

    }

    #[test]
    fn test_extract_redundancy_count() {
        let label = "Redundancy *4";
        assert_eq!(extract_redundancy_count(label), 4);

        // Test default redundancy
        let label_missing_redundancy = "No redundancy mentioned";
        assert_eq!(extract_redundancy_count(label_missing_redundancy), 1);
    }

    #[test]
    fn test_extract_module_name() {
        let label = "mod::MyModule";
        assert_eq!(extract_module_name("NodeA", label), "MyModule");

        // Test default module name based on node ID
        let label_missing_module = "No module here";
        assert_eq!(extract_module_name("NodeA", label_missing_module), "mod_node_a");
    }

    #[test]
    fn test_extract_consume_pattern_from_label() {
        let label_peek_copy = ">>PeekCopy something else";
        assert_eq!(extract_consume_pattern_from_label(label_peek_copy), ConsumePattern::PeekCopy);

        let label_take = ">>Take even more";
        assert_eq!(extract_consume_pattern_from_label(label_take), ConsumePattern::Take);

        // Test default consume pattern
        let label_missing_pattern = "No pattern here";
        assert_eq!(extract_consume_pattern_from_label(label_missing_pattern), ConsumePattern::Take);
    }

    #[test]
    fn test_extract_actor_driver_from_label() {
        let label = "AtLeastEvery(5000ms) && OnEvent(C1//10||B2//10) && OnCapacity(C2//20||A1//20)";
        let drivers = extract_actor_driver_from_label(label);
        // This would check for the presence and correctness of each driver type
        // This example assumes you have PartialEq derived for your ActorDriver and other types for simplicity
        assert!(drivers.contains(&ActorDriver::AtLeastEvery(Duration::from_millis(5000))));
        assert!(drivers.iter().any(|d| matches!(d, ActorDriver::EventDriven(_))));
        assert!(drivers.iter().any(|d| matches!(d, ActorDriver::CapacityDriven(_))));
    }
}


