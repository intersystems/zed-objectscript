use crate::common::{
    get_node_children, get_string_at_byte_range, get_tracked_keywords, parse_return_type,
};
use crate::parse_structures::{Cardinality, Relationship};
use tree_sitter::Node;

impl Default for Relationship {
    fn default() -> Self {
        Self {
            required: false,
            is_final: None,
            is_public: true,
            name: "TODO".to_string(),
            cardinality: Cardinality::One,
            return_type: None,
            inverse: "TODO".to_string(),
        }
    }
}
pub fn build_relationship_struct(relationship_node: Node, content: &str) -> Option<Relationship> {
    if relationship_node.kind() != "relationship" {
        eprintln!(
            "Error: build_relationship_struct was called for node {:?}, but it can only be called for relationship nodes",
            relationship_node.kind()
        );
        return None;
    }
    let mut relationship = Relationship::default();
    let relationship_children = get_node_children(relationship_node);
    for relationship_child in relationship_children {
        match relationship_child.kind() {
            "relationship_name" => {
                if let Some(relationship_name_node) = relationship_child.named_child(0)
                    && let Some(name) =
                        get_string_at_byte_range(content, relationship_name_node.byte_range())
                {
                    relationship.name = name;
                } else {
                    eprintln!("Error: failed to get relationship name.");
                    return None;
                }
            }
            "return_type" => {
                relationship.return_type = parse_return_type(relationship_child, content);
            }
            "relationship_keywords" => {
                let tracked_keywords = get_tracked_keywords(relationship_child, content);
                if let Some(cardinality) = tracked_keywords.cardinality
                    && let Some(inverse) = tracked_keywords.inverse
                {
                    relationship.cardinality = cardinality;
                    relationship.inverse = inverse;
                } else {
                    eprintln!(
                        "Error: Relationship requires cardinality and inverse keywords to be defined"
                    );
                    return None;
                }
                relationship.is_final = tracked_keywords.is_final;
                relationship.required = tracked_keywords.is_required;
                relationship.is_public = tracked_keywords.is_public;
            }
            "keyword_relationship" => {}
            _ => {
                eprintln!(
                    "Error: Unrecognized relationship child node {:?}",
                    relationship_child.kind()
                );
                continue;
            }
        }
    }
    if &relationship.name == "TODO" || &relationship.inverse == "TODO" {
        eprintln!(
            "error: failed to parse relationship name {:?} or inverse keyword {:?}",
            &relationship.name, &relationship.inverse
        );
        return None;
    }
    Some(relationship)
}
