use crate::common::{
    build_argument, get_node_children, get_string_at_byte_range, get_tracked_keywords,
    parse_return_type,
};
use crate::parse_structures::Property;
use std::collections::HashMap;
use tree_sitter::Node;

impl Default for Property {
    fn default() -> Self {
        Self {
            required: false,
            is_final: None,
            is_public: true,
            name: "TODO".to_string(),
            multidimensional: false,
            return_type: None,
            arguments: HashMap::new(),
            next_argument_id: 0,
        }
    }
}

pub fn build_property_struct(property_node: Node, content: &str) -> Option<Property> {
    if property_node.kind() != "property" {
        eprintln!(
            "Error: build_property_struct was called for node {:?}, but it can only be called for property nodes",
            property_node.kind()
        );
        return None;
    }
    let mut property = Property::default();
    let property_children = get_node_children(property_node);
    for property_child in property_children {
        match property_child.kind() {
            "property_name" => {
                if let Some(property_name_node) = property_child.named_child(0)
                    && let Some(name) =
                        get_string_at_byte_range(content, property_name_node.byte_range())
                {
                    property.name = name;
                } else {
                    eprintln!("Error: failed to get property name.");
                    return None;
                }
            }
            "arguments" => {
                let argument_children = get_node_children(property_child);
                for argument in argument_children {
                    if let Some((argument_struct, argument_range)) =
                        build_argument(argument, content)
                    {
                        property.arguments.insert(
                            argument_struct.name.clone(),
                            (argument_struct, argument_range),
                        );
                    }
                }
            }
            "return_type" => {
                property.return_type = parse_return_type(property_child, content);
            }
            "property_keywords" => {
                let tracked_keywords = get_tracked_keywords(property_child, content);
                property.is_final = tracked_keywords.is_final;
                property.multidimensional = tracked_keywords.multidimensional;
                property.required = tracked_keywords.is_required;
                property.is_public = tracked_keywords.is_public;
            }
            _ => continue,
        }
    }
    if &property.name == "TODO" {
        eprintln!("error: failed to parse property name");
        return None;
    }
    Some(property)
}
