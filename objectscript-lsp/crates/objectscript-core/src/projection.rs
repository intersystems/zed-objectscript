use crate::common::{
    get_node_children, get_string_at_byte_range, get_tracked_keywords, parse_return_type,
};
use crate::parse_structures::Projection;
use tree_sitter::Node;

pub fn build_projection_struct(projection_node: Node, content: &str) -> Option<Projection> {
    if projection_node.kind() != "projection" {
        eprintln!(
            "Error: build_projection_struct was called for node {:?}, but it can only be called for projection nodes",
            projection_node.kind()
        );
        return None;
    }
    let mut is_final = None;
    let mut projection_name = None;
    let mut return_type = None;
    let projection_children = get_node_children(projection_node);
    for projection_child in projection_children {
        match projection_child.kind() {
            "projection_name" => {
                if let Some(projection_name_node) = projection_child.named_child(0) {
                    projection_name =
                        get_string_at_byte_range(content, projection_name_node.byte_range());
                } else {
                    eprintln!("Error: failed to get projection name.");
                    return None;
                }
            }
            "return_type" => {
                return_type = parse_return_type(projection_child, content);
            }
            "projection_keywords" => {
                let tracked_keywords = get_tracked_keywords(projection_child, content);
                is_final = tracked_keywords.is_final;
            }
            _ => {
                eprintln!(
                    "Error: Unrecognized projection child node {:?}",
                    projection_child.kind()
                );
                continue;
            }
        }
    }
    if let Some(name) = projection_name
        && let Some(return_type) = return_type
    {
        return Some(Projection {
            name,
            is_final,
            return_type,
        });
    }

    return None;
}
