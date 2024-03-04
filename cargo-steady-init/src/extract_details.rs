use dot_parser::canonical::Graph;
use std::error::Error;
use std::ops::Index;
use crate::ProjectModel;

pub(crate) fn extract_type_name_from_edge_label(label_text: &str, from_node: &str, to_node: &str) -> Option<String> {
    ///////////////////////////////////
    // Attempt to find the type name
    // This is preferred if it exists
    //////////////////////////////////
    if let (Some(start), Some(end)) = (label_text.find('<'), label_text.find('>')) {
        if start < end {
            let type_name = &label_text[start + 1..end];
            // Check if the extracted part has no whitespace and starts with a capital letter
            if type_name.chars().all(|ch| !ch.is_whitespace()) && type_name.chars().next()?.is_uppercase() {
                return Some(type_name.to_string());
            }
        }
    }

    ////////////////////////////////////////////////////////////////////////////////
    //after this point we will attempt to select something helpful from the label
    ////////////////////////////////////////////////////////////////////////////////
    // If the first attempt fails, split by newline and process the first line
    let input = label_text.replace("\\n","\n").replace("\"","");
    let first_line = input.lines().next();
    if let Some(first_line) = first_line {

        let parts: Vec<String> = first_line.split_whitespace()
            .map(|s| {
                let mut chars = s.chars();
                match chars.next() {
                    Some(first_char) => first_char.to_uppercase().collect::<String>() + chars.as_str(),
                    None => s.to_string(),
                }
            })
            .collect();
        if let Some(x) = parts.iter().next() {
            if x.len() > 0 {
                let joined = parts.join("");
                return Some(joined);
            }
        }
    }
    //if there is no label then we build up a type based on from and to nodes
    Some(format!("From{}To{}",from_node,to_node).to_string())
}

#[cfg(test)]
mod tests {
    use crate::extract_details;

    #[test]
    fn test_extract_type_name_from_edge_label() {
        let label = "IMAP server details\nemail, password";
        let from = "ConfigLoader";
        let to = "IMAPClient";
        let result = extract_details::extract_type_name_from_edge_label(label, from, to);
        assert_eq!(result, Some("IMAPServerDetails".to_string()));
    }
}

pub(crate) fn extract_project_model<'a, A>(name: &str, g: Graph<'a, A>) -> Result<ProjectModel, Box<dyn Error>>
  where A:  Copy {
    let mut pm = ProjectModel::default();
    pm.name = name.to_string();


       g.nodes.set.iter().for_each(|node| {
           let att = node.1.attr.elems.len();
           let id = node.1.id;
           println!("Node: {:?}", id);
           //TODO: each of those must be mod files
           //      also created in the graph

       });
/*
       canonical.edges.set.iter().for_each(|e| {
           let to = e.to;
           let from = e.from;

           e.attr.elems.iter().for_each(|(a,b)| {
               if "label".eq(*a) {
                   let type_name = extract_details::extract_type_name_from_edge_label(b,from,to);
                   println!("TypeName: {:?}",type_name);
               }

           });


           println!("Edge: {:?}->{:?} ", from, to);
           //TODO: each of those must use builder to define channel
           //      each of these may or may not be the same type



       });
  */


    Ok(pm)
}
