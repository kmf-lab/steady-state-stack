use std::cell::RefCell;
use dot_parser::canonical::Graph;
use std::error::Error;
use std::time::Duration;
use log::{error, warn};
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
                let contains_colon  = s.contains(":");
                let mut chars = s.chars();
                match chars.next() {
                    Some(first_char) =>
                        {
                            if first_char.is_alphabetic() && !contains_colon {
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



fn extract_module_name(node_id: &str, label_text: &str) -> String {
    let module_prefix = "mod::";
    if let Some(start) = label_text.find(module_prefix) {
        let remaining = &label_text[start + module_prefix.len()..];
        // Find the first occurrence of either a comma or a whitespace character or escape \n etc
        if let Some(end) = remaining.find(|c: char| c == ',' || c.is_whitespace() || c == '\\') {
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
            if let Some(parts) = parse_parts(part, "OnEvent") {
                Some(ActorDriver::EventDriven(parts))
            } else {
                warn!("Failed to parse OnEvent driver: {}", part);
                let _ = parse_parts(part, "OnEvent");

                None
            }
        } else if part.starts_with("OnCapacity") {
            if let Some(parts) = parse_parts(part, "OnCapacity") {
                Some(ActorDriver::CapacityDriven(parts))
            } else {
                warn!("Failed to parse OnCapacity driver: {}", part);
                None
            }


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

/// will parse out strings like Event(xx:1||y:1) and Capacity(xx:1||y:1)
/// where prefix is "Event" or "Capacity"

fn parse_parts(part: &str, prefix: &str) -> Option<Vec<Vec<String>>> {
    part.strip_prefix(prefix)
        .and_then(|s| s.split_once('('))
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(content,_)| {
                let v: Vec<Vec<String>> = content.split("||").filter_map(|text| {
                    if text.is_empty() {
                        None
                    } else {
                        let parts: Vec<_> = text.split(':').map(|s| s.to_string()).collect();
                        Some(parts)
                    }
                }).collect();
                v
            }
        ).filter(|v| !v.is_empty())
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
pub(crate) fn extract_project_model(name: &str, dot_graph: Graph<(String, String)>) -> Result<ProjectModel, Box<dyn Error>> {

    let empty = "".to_string();
    // Iterate over nodes to populate actors
    let nodes:Vec<(&str,&str)> = dot_graph.nodes.set.iter()
                            .map(|node| (node.0, node.1.attr.elems.iter()
                                              .find_map(|(key, value)| if "label".eq(key) { Some(value) } else { None }).unwrap_or(&empty)
                                         )
                                )
                            .map(|(a,b)| (a.as_str(),b.as_str()))
                            .filter(|(a,b)| !a.starts_with("__"))
                            .collect();


    let edges:Vec<(&str,&str,&str)> = dot_graph.edges.set.iter()
        .map(|edge| (&edge.from, &edge.to, edge.attr.elems
            .iter()
            .find_map(|(key, value)| if "label".eq(key) { Some(value) } else { None }).unwrap_or(&empty)
        ))
        .map(|(a,b,c)| (a.as_str(),b.as_str(),c.as_str()))
        .collect();


    build_pm(ProjectModel { name: name.to_string(), ..Default::default() }, nodes, edges)
}

fn build_pm(mut pm: ProjectModel, mut nodes: Vec<(&str, &str)>, mut edges: Vec<(&str, &str, &str)>) -> Result<ProjectModel, Box<dyn Error>> {

    nodes.sort(); //to ensure we get the same results on each run
    for (id, label_text) in nodes {
        let mod_name = extract_module_name(id, label_text);

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

    edges.sort(); //to ensure we get the same results on each run
    // Iterate over edges to populate channels
    for (from, to, label_text) in edges {
        let type_name = extract_type_name_from_edge_label(label_text, from, to);
        let capacity = extract_capacity_from_edge_label(label_text, 8);  // Assuming 8 as default if not specified


        let name = extract_channel_name(label_text, from, to);
        let consume_pattern = extract_consume_pattern_from_label(label_text);

        if let Some(mod_name) = pm.actors.iter().filter(|f| f.display_name.eq(from)).map(|a| a.mod_name.clone()).next() {
            let to_mod = pm.actors.iter().filter(|f| f.display_name.eq(to)).map(|a| a.mod_name.clone()).next().unwrap_or("unknown".into());

            // Create a Channel instance based on extracted details
            let mut channel = Channel {
                name,
                from_mod: mod_name,
                to_mod,
                batch_read: 1,  //default replaced later if set
                batch_write: 1,  //default replaced later if set
                message_type: type_name,
                peek: consume_pattern == ConsumePattern::PeekCopy,
                copy: consume_pattern != ConsumePattern::Take,
                capacity,
                bundle_index: -1,
                rebundle_index: -1,
                to_node: to.into(),
                from_node: from.into(),
                bundle_on_from: RefCell::new(true),
                is_unbundled: false,
            };

            // Find the actor with the same id as the from node and add the channel to its tx_channels
            if let Some(a) = pm.actors.iter_mut().find(|f| f.display_name.eq(from)) {
                //we found the actor which is the source of this channel

                //review this actor and see if it has some batched write size
                a.driver.iter().for_each(|f| {
                    if let ActorDriver::CapacityDriven(pairs) = f {
                        pairs.iter()
                            .filter(|v| v[0].eq(&channel.name))
                            .for_each(|v| channel.batch_write = v[1].parse().expect("expected int"));
                    };
                });

                let mut tx_channel = channel.clone();

                //count the number of channels with the same name and tx_node ie the rx end
                let tx_counter_index = pm.channels.iter().filter(|f|
                f[0].name.eq(&channel.name)
                ).map(|f| f.iter()
                    .filter(|f| f.to_node.eq(&tx_channel.to_node))
                    .count()).max().unwrap_or(0);

                tx_channel.bundle_index = tx_counter_index as isize;
                //re-bundle is NOT set since we only use that feature for the Rx end


                // tx_channel.bundle_on_from=false; //bundle on 'to' because we tx to there

                roll_up_bundle(&mut a.tx_channels, tx_channel, |_t, _v| true);
            }
            if let Some(a) = pm.actors.iter_mut().find(|f| f.display_name.eq(to)) {
                //we found the actor which is the source of this channel

                //review this actor and see if it has some batched write size
                a.driver.iter().for_each(|f| {
                    if let ActorDriver::EventDriven(pairs) = f {
                        pairs.iter()
                            .filter(|v| v[0].eq(&channel.name))
                            .for_each(|v| channel.batch_read = v[1].parse().expect("expected int"));
                    };
                });
                let mut rx_channel = channel.clone();

                //count the number of channels with the same name and from_node ie the tx end
                let rx_counter_index = pm.channels.iter().filter(|f|
                f[0].name.eq(&channel.name)
                ).map(|f| f.iter()
                    .filter(|f| f.from_node.eq(&rx_channel.from_node))
                    .count()).max().unwrap_or(0);
                rx_channel.rebundle_index = rx_counter_index as isize; //for building dynamic bundle if needed
                rx_channel.bundle_index = rx_counter_index as isize; //for just indexing into the bundle if needed

                roll_up_bundle(&mut a.rx_channels, rx_channel, |_t, _v| true);
            }
            // these are rolled up to define each bundle at the top of main
            // at that point all channels are gathered by name and source node
            roll_up_bundle(&mut pm.channels, channel, |t, v| v.iter().all(|g| g.from_node.eq(&t.from_node)));
            //after that point we may reassemble the targets into new bundles.
        } else {
            error!("Failed to find actor with id: {}", from);
        }
    }
    //now that all bundle_on_from are detected look at the remaining channels to find any
    //that are bundles on the 'to' side and move them to the new group
    //walk pm.channels and if they are bundles of len()>1 move them to the new group if not
    //for each single call roll_up_bundle on to_node. this way we select any and all from bundles first
    let mut new_channels: Vec<Vec<Channel>> = Vec::new();
    pm.channels.into_iter().for_each(|mut c| {
        if c.len() > 1 {
            //keep existing bundles based on from
            new_channels.push(c);
        } else if let Some(local) = c.pop() {
            //take each single and see if we can roll them up based on to_node
            roll_up_bundle(&mut new_channels, local, |t, v| {
                let result = v.iter().all(|g| g.to_node.eq(&t.to_node));
                if result {
                    //success we found a to_node bundle so mark the members as such
                    v.iter().for_each(|f| { f.bundle_on_from.replace(false); });
                }
                result
            });
        }
    });
    pm.channels = new_channels;


    // we need to post process the channels now that we know which are bundles
    // find all the non bundles in the main channels list
    pm.channels.iter().for_each(|c| {
        //find the actor that has this channel in its tx_channels
        if let Some(a) = pm.actors.iter_mut().find(|f| f.display_name.eq(&c[0].from_node)) {
            //find the channel in the tx_channels and mark it as a bundle
            if let Some(x) = a.tx_channels.iter_mut().find(|f| f[0].name.eq(&c[0].name)) {
                x.iter_mut().for_each(|f| {
                    f.bundle_on_from.clone_from(&c[0].bundle_on_from);
                    f.is_unbundled = c.len() == 1;
                });
            }
        }
        //find the actor that has this channel in its rx_channels
        if let Some(a) = pm.actors.iter_mut().find(|f| f.display_name.eq(&c[0].to_node)) {
            //find the channel in the rx_channels and mark it as a bundle
            if let Some(x) = a.rx_channels.iter_mut().find(|f| f[0].name.eq(&c[0].name)) {
                x.iter_mut().for_each(|f| {
                    f.bundle_on_from.clone_from(&c[0].bundle_on_from);
                    f.is_unbundled = c.len() == 1;
                });
            }
        }
    });


    Ok(pm)
}
/*
(do_not_group_by
|| f.iter().all(|g| g.from_node.eq(&insert_me.from_node) )
|| f.iter().all(|g| g.to_node.eq(&insert_me.to_node))
)*/

/// This function is used to roll up channels into bundles and is important for the code generation
/// Some Channels are grouped into vecs because they are all the same and either originate
/// or terminate at the same actor. This simplifies code to allow for indexing of channels.
fn roll_up_bundle(collection: &mut Vec<Vec<Channel>>, mut insert_me: Channel, group_by: fn(&Channel, &Vec<Channel>) -> bool) {

        if let Some(x) = collection.iter_mut().find(|f| {

            f[0].name.eq(&insert_me.name) // we all agree on the name
                && f[0].capacity.eq(&insert_me.capacity) // we all agree on the capacity
                && f[0].message_type.eq(&insert_me.message_type) // we should all agree on the type
                && group_by(&insert_me, f)
        }

        ) { //this is clearly part of the same bundle
            //before doing the push we need to confirm we all have the same capacity and from_mod

            //update all others to our desired greater capacity, ie all are expanded to match the longest
            if insert_me.capacity > x[0].capacity {
                x.iter_mut().for_each(|f| f.capacity = insert_me.capacity );
            } else {
                insert_me.capacity = x[0].capacity;
            }
            //update all others to match the first from_mod used
            if !insert_me.from_mod.eq(&x[0].from_mod) {
                insert_me.from_mod.clone_from(&x[0].from_mod);
            }
            //update all to match the copy boolean in the bundle
            if insert_me.copy != x[0].copy {
                if x[0].copy {
                    insert_me.copy = true;
                } else {
                    x.iter_mut().for_each(|f| f.copy = true );
                }
            }
            x.push(insert_me); //keep our re-bundle_index unchanged
            //restore all to -1 since we now know this is a bundle for sure.
            x.iter_mut().for_each(|f| f.bundle_index = -1 );

        } else {
            collection.push(vec![insert_me]);
        }



}
////////
///////


#[cfg(test)]
mod tests {
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



    #[test]
    fn test_correct_format() {
        let input = "Event(xx:1||y:1)";
        let expected = Some(vec![vec!["xx".to_string(), "1".to_string()], vec!["y".to_string(), "1".to_string()]]);
        assert_eq!(parse_parts(input, "Event"), expected);
    }

    #[test]
    fn test_multiple_parts() {
        let input = "Capacity(a:1:b:2||c:3:d:4)";
        let expected = Some(vec![vec!["a".to_string(), "1".to_string(), "b".to_string(), "2".to_string()], vec!["c".to_string(), "3".to_string(), "d".to_string(), "4".to_string()]]);
        assert_eq!(parse_parts(input, "Capacity"), expected);
    }

    #[test]
    fn test_incorrect_prefix() {
        let input = "Event(xx:1||y:1)";
        assert_eq!(parse_parts(input, "Capacity"), None);
    }

    #[test]
    fn test_missing_closing_parenthesis() {
        let input = "Event(xx:1||y:1";
        assert_eq!(parse_parts(input, "Event"), None);
    }

    #[test]
    fn test_empty_content() {
        let input = "Event()";
        assert_eq!(parse_parts(input, "Event"), None);
    }

    #[test]
    fn test_no_delimiters() {
        let input = "Event(xxy1)";
        let expected = Some(vec![vec!["xxy1".to_string()]]);
        assert_eq!(parse_parts(input, "Event"), expected);
    }

    #[test]
    fn test_example_1() {
        let input = "OnEvent(client_request:1||feedback:1)";
        let expected = Some(vec![
            vec!["client_request".to_string(),"1".to_string()],
            vec!["feedback".to_string(),"1".to_string()]
        ]);
        assert_eq!(parse_parts(input, "OnEvent"), expected);
    }

    #[test]
    fn test_example_2() {
        let input = "OnEvent(pbft_message:1)";
        let expected = Some(vec![vec!["pbft_message".to_string(),"1".to_string()]]);
        assert_eq!(parse_parts(input, "OnEvent"), expected);
    }

}

#[cfg(test)]
mod additional_tests {
    use super::*;
    use std::error::Error;
    use dot_parser::canonical::{Graph, Node, Edge};
    use crate::templates::{Actor, Channel, ActorDriver, ConsumePattern};

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("CamelCase"), "camel_case");
        assert_eq!(to_snake_case("camelCase"), "camel_case");
        assert_eq!(to_snake_case("CamelCaseWithMultipleWords"), "camel_case_with_multiple_words");
        assert_eq!(to_snake_case("already_snake_case"), "already_snake_case");
    }

    #[test]
    fn test_find_start_position() {
        let label = "AtLeastEvery(100ms) && OnEvent(C1//10||B2//10)";
        assert_eq!(find_start_position(label), 0);

        let label = "Some text before AtMostEvery(500ms) && OnEvent(C1//10||B2//10)";
        assert_eq!(find_start_position(label), 17);

        let label = "No matching keywords";
        assert_eq!(find_start_position(label), label.len());
    }

    #[test]
    fn test_extract_channel_name() {
        let label = "name::MyChannelName";
        assert_eq!(extract_channel_name(label, "NodeA", "NodeB"), "MyChannelName");

        let label = "No name here";
        assert_eq!(extract_channel_name(label, "NodeA", "NodeB"), "node_a_to_node_b");
    }


    #[test]
    fn test_extract_capacity_from_edge_label_edge_cases() {
        let label = "Capacity #1024extra";
        assert_eq!(extract_capacity_from_edge_label(label, 512), 1024);

        let label = "Capacity #invalid";
        assert_eq!(extract_capacity_from_edge_label(label, 512), 512);
    }

    #[test]
    fn test_extract_type_name_from_edge_label() {
        assert_eq!(extract_type_name_from_edge_label("<TypeName>", "from_node", "to_node"), "TypeName");
    //    assert_eq!(extract_type_name_from_edge_label("label text", "from_node", "to_node"), "Fromfrom_nodeTo_to_node");
    //    assert_eq!(extract_type_name_from_edge_label("", "from_node", "to_node"), "Fromfrom_nodeTo_to_node");
    }

    #[test]
    fn test_extract_capacity_from_edge_label() {
        assert_eq!(extract_capacity_from_edge_label("#10", 8), 10);
        assert_eq!(extract_capacity_from_edge_label("label text", 8), 8);
        assert_eq!(extract_capacity_from_edge_label("", 8), 8);
    }

    #[test]
    fn test_extract_module_name() {
        assert_eq!(extract_module_name("node_id", "mod::module_name"), "module_name");
        assert_eq!(extract_module_name("node_id", "label text"), "mod_node_id");
        assert_eq!(extract_module_name("node_id", ""), "mod_node_id");
    }



    #[test]
    fn test_extract_consume_pattern_from_label() {
        assert_eq!(extract_consume_pattern_from_label(">>PeekCopy"), ConsumePattern::PeekCopy);
        assert_eq!(extract_consume_pattern_from_label(">>TakeCopy"), ConsumePattern::TakeCopy);
        assert_eq!(extract_consume_pattern_from_label(">>Take"), ConsumePattern::Take);
        assert_eq!(extract_consume_pattern_from_label(""), ConsumePattern::Take);
    }



    #[test]
    fn test_extract_actor_driver_from_label() {
        assert_eq!(extract_actor_driver_from_label("AtMostEvery(10ms)"), vec![ActorDriver::AtMostEvery(Duration::from_millis(10))]);
        assert_eq!(extract_actor_driver_from_label("AtLeastEvery(10ms)"), vec![ActorDriver::AtLeastEvery(Duration::from_millis(10))]);
        assert_eq!(extract_actor_driver_from_label("OnEvent(event:1)"), vec![ActorDriver::EventDriven(vec![vec!["event".to_string(), "1".to_string()]])]);
        assert_eq!(extract_actor_driver_from_label("OnCapacity(capacity:1)"), vec![ActorDriver::CapacityDriven(vec![vec!["capacity".to_string(), "1".to_string()]])]);
        assert_eq!(extract_actor_driver_from_label("Other(item1,item2)"), vec![ActorDriver::Other(vec!["item1".to_string(), "item2".to_string()])]);
        assert_eq!(extract_actor_driver_from_label(""), vec![ActorDriver::AtMostEvery(Duration::from_secs(1))]);
    }

    #[test]
    fn test_parse_parts() {
        assert_eq!(parse_parts("Event(event:1||event2:2)", "Event"), Some(vec![vec!["event".to_string(), "1".to_string()], vec!["event2".to_string(), "2".to_string()]]));
        assert_eq!(parse_parts("Capacity(capacity:1||capacity2:2)", "Capacity"), Some(vec![vec!["capacity".to_string(), "1".to_string()], vec!["capacity2".to_string(), "2".to_string()]]));
        assert_eq!(parse_parts("Event(event:1)", "Event"), Some(vec![vec!["event".to_string(), "1".to_string()]]));
        assert_eq!(parse_parts("", "Event"), None);
    }

    #[test]
    fn test_build_pm() {
        let mut pm = ProjectModel::default();
        let nodes = vec![("node1", "label1"), ("node2", "label2")];
        let edges = vec![("node1", "node2", "edge_label")];

        let result = build_pm(pm, nodes, edges).unwrap();
        assert_eq!(result.actors.len(), 2);
        assert_eq!(result.channels.len(), 1);
    }

    #[test]
    fn test_build_pm_empty_edges() {
        let mut pm = ProjectModel::default();
        let nodes = vec![("node1", "label1"), ("node2", "label2")];
        let edges = vec![];

        let result = build_pm(pm, nodes, edges).unwrap();
        assert_eq!(result.actors.len(), 2);
        assert_eq!(result.channels.len(), 0);
    }

    #[test]
    fn test_roll_up_bundle() {
        let mut channels = vec![];
        let channel = Channel {
            name: "channel_name".to_string(),
            from_mod: "from_mod".to_string(),
            to_mod: "to_mod".to_string(),
            batch_read: 1,
            batch_write: 1,
            message_type: "message_type".to_string(),
            peek: false,
            copy: false,
            capacity: 10,
            bundle_index: -1,
            rebundle_index: -1,
            to_node: "to_node".to_string(),
            from_node: "from_node".to_string(),
            bundle_on_from: RefCell::new(true),
            is_unbundled: false,
        };

        roll_up_bundle(&mut channels, channel, |_t, _v| true);
        assert_eq!(channels.len(), 1);
    }

    #[test]
    fn test_roll_up_bundle_empty_channels() {
        let mut channels = vec![];
        let channel = Channel {
            name: "channel_name".to_string(),
            from_mod: "from_mod".to_string(),
            to_mod: "to_mod".to_string(),
            batch_read: 1,
            batch_write: 1,
            message_type: "message_type".to_string(),
            peek: false,
            copy: false,
            capacity: 10,
            bundle_index: -1,
            rebundle_index: -1,
            to_node: "to_node".to_string(),
            from_node: "from_node".to_string(),
            bundle_on_from: RefCell::new(true),
            is_unbundled: false,
        };

        roll_up_bundle(&mut channels, channel, |_t, _v| true);
        assert_eq!(channels.len(), 1);
    }

    // #[test]
    // fn test_roll_up_bundle_invalid_predicate() {
    //     let mut channels = vec![];
    //     let channel = Channel {
    //         name: "channel_name".to_string(),
    //         from_mod: "from_mod".to_string(),
    //         to_mod: "to_mod".to_string(),
    //         batch_read: 1,
    //         batch_write: 1,
    //         message_type: "message_type".to_string(),
    //         peek: false,
    //         copy: false,
    //         capacity: 10,
    //         bundle_index: -1,
    //         rebundle_index: -1,
    //         to_node: "to_node".to_string(),
    //         from_node: "from_node".to_string(),
    //         bundle_on_from: RefCell::new(true),
    //         is_unbundled: false,
    //     };
    //
    //     roll_up_bundle(&mut channels, channel, |_t, _v| false);
    //     assert_eq!(channels.len(), 0);
    // }



}


