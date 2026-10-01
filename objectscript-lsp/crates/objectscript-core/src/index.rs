use crate::common::{
    find_return_type, get_node_children, get_string_at_byte_range, get_tracked_keywords,
    parse_return_type,
};
use crate::parse_structures::{Index, IndexPropertyValue, IndexType, TypeName};
use tree_sitter::Node;

impl Default for Index {
    fn default() -> Self {
        Self {
            name: "TODO".to_string(),
            properties: Vec::new(),
            index_type: IndexType::Index,
            return_type: None,
        }
    }
}
pub fn build_index_struct(index_node: Node, content: &str) -> Option<Index> {
    if index_node.kind() != "index" {
        eprintln!(
            "Error: build_index_struct was called for node {:?}, but it can only be called for index nodes",
            index_node.kind()
        );
        return None;
    }
    let mut index = Index::default();
    let index_children = get_node_children(index_node);
    for index_child in index_children {
        match index_child.kind() {
            "index_name" => {
                if let Some(index_name_node) = index_child.named_child(0)
                    && let Some(index_name) =
                        get_string_at_byte_range(content, index_name_node.byte_range())
                {
                    index.name = index_name;
                } else {
                    eprintln!("Error: failed to get index name.");
                    return None;
                }
            }
            "return_type" => {
                index.return_type = parse_return_type(index_child, content);
            }
            "index_property_value" => {
                let mut property_name = None;
                let mut elements = false;
                let mut keys = false;
                let mut property_return_type = None;
                let index_property_value_children = get_node_children(index_child);
                for index_property_value_child in index_property_value_children {
                    match index_property_value_child.kind() {
                        "property_name" => {
                            if let Some(property_name_node) =
                                index_property_value_child.named_child(0)
                            {
                                property_name = get_string_at_byte_range(
                                    content,
                                    property_name_node.byte_range(),
                                );
                            } else {
                                eprintln!("Error: failed to get property name.");
                                continue;
                            }
                        }
                        "index_type" => {
                            let mut return_type_parameters = Vec::new();
                            let mut return_type_id = None;
                            let index_type_children = get_node_children(index_property_value_child);
                            for index_type_child in index_type_children {
                                match index_type_child.kind() {
                                    "typename" => {
                                        if let Some(typename_identifier) = get_string_at_byte_range(
                                            content,
                                            index_type_child.byte_range(),
                                        ) {
                                            return_type_id =
                                                Some(find_return_type(typename_identifier));
                                        }
                                    }
                                    "numeric_literal" => {
                                        if let Some(param) = get_string_at_byte_range(
                                            content,
                                            index_type_child.byte_range(),
                                        ) {
                                            return_type_parameters.push(param);
                                        }
                                    }
                                    _ => continue,
                                }
                            }
                            if let Some(return_type_id) = return_type_id {
                                property_return_type = Some(TypeName {
                                    ret_type: return_type_id,
                                    parameters: return_type_parameters,
                                })
                            }
                        }
                        "typename" => {
                            if let Some(val) = get_string_at_byte_range(
                                content,
                                index_property_value_child.byte_range(),
                            ) {
                                if val.to_ascii_lowercase() == "elements" {
                                    elements = true;
                                }
                                if val.to_ascii_lowercase() == "keys" {
                                    keys = true;
                                }
                            }
                        }
                        _ => continue,
                    }
                }
                if let Some(property_name) = property_name {
                    index.properties.push(IndexPropertyValue {
                        name: property_name,
                        elements,
                        keys,
                        return_type: property_return_type,
                    })
                }
            }
            "index_keywords" => {
                let tracked_keywords = get_tracked_keywords(index_child, content);
                index.index_type = tracked_keywords.index_type;
            }
            "extent_index_keywords" => {
                index.index_type = IndexType::Extent;
            }
            _ => {
                continue;
            }
        }
    }

    if &index.name == "TODO" {
        return None;
    }
    Some(index)
}
