use crate::common::{
    build_argument, get_node_children, get_string_at_byte_range, get_tracked_keywords,
    parse_return_type,
};
use crate::parse_structures::Query;
use std::collections::HashMap;
use tree_sitter::Node;

pub fn build_query_struct(query_node: Node, content: &str) -> Option<Query> {
    if query_node.kind() != "query" {
        eprintln!(
            "Error: build_query_struct was called for node {:?}, but it can only be called for query nodes",
            query_node.kind()
        );
        return None;
    }
    let mut required_privileges = Vec::new();
    let mut is_final = None;
    let mut is_public = true;
    let mut query_name = None;
    let mut return_type = None;
    let mut arguments = HashMap::new();
    let query_children = get_node_children(query_node);
    for query_child in query_children {
        match query_child.kind() {
            "query_name" => {
                if let Some(query_name_node) = query_child.named_child(0) {
                    query_name = get_string_at_byte_range(content, query_name_node.byte_range());
                } else {
                    eprintln!("Error: failed to get query name.");
                    return None;
                }
            }
            "return_type" => {
                return_type = parse_return_type(query_child, content);
            }
            "query_keywords" => {
                let tracked_keywords = get_tracked_keywords(query_child, content);
                is_final = tracked_keywords.is_final;
                required_privileges = tracked_keywords.requires;
                is_public = tracked_keywords.is_public;
            }
            "keyword_query" | "external_method_body_content" => {}
            "arguments" => {
                let argument_children = get_node_children(query_child);
                for argument in argument_children {
                    if let Some((argument_struct, argument_range)) =
                        build_argument(argument, content)
                    {
                        arguments.insert(
                            argument_struct.name.clone(),
                            (argument_struct, argument_range),
                        );
                    }
                }
            }
            _ => {
                eprintln!(
                    "Error: Unrecognized query child node {:?}",
                    query_child.kind()
                );
                continue;
            }
        }
    }
    if let Some(name) = query_name
        && let Some(return_type) = return_type
    {
        return Some(Query {
            required_privileges,
            is_final,
            is_public,
            name,
            return_type,
            arguments,
        });
    }

    return None;
}
