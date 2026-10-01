use crate::common::{get_node_children, get_string_at_byte_range, get_tracked_keywords};
use crate::parse_structures::{ForeignKey, ForeignKeyAction};
use tree_sitter::Node;

impl Default for ForeignKey {
    fn default() -> Self {
        Self {
            name: "TODO".to_string(),
            properties_constrained: Vec::new(),
            referenced_class: "TODO".to_string(),
            referenced_index: None,
            on_delete: ForeignKeyAction::NoAction,
            on_update: ForeignKeyAction::NoAction,
        }
    }
}

pub fn build_foreignkey_struct(foreignkey_node: Node, content: &str) -> Option<ForeignKey> {
    if foreignkey_node.kind() != "foreignkey" {
        eprintln!(
            "Error: build_foreignkey_struct was called for node {:?}, but it can only be called for foreignkey nodes",
            foreignkey_node.kind()
        );
        return None;
    }
    let mut foreign_key = ForeignKey::default();
    let foreign_key_children = get_node_children(foreignkey_node);
    for foreign_key_child in foreign_key_children {
        match foreign_key_child.kind() {
            "foreignkey_name" => {
                if let Some(foreignkey_name_node) = foreign_key_child.named_child(0)
                    && let Some(name) =
                        get_string_at_byte_range(content, foreignkey_name_node.byte_range())
                {
                    foreign_key.name = name;
                } else {
                    eprintln!("Error: failed to get foreignkey name.");
                    return None;
                }
            }
            "property_name" => {
                if let Some(property_name_node) = foreign_key_child.named_child(0)
                    && let Some(property_name) =
                        get_string_at_byte_range(content, property_name_node.byte_range())
                {
                    foreign_key.properties_constrained.push(property_name);
                }
            }
            "class_name" => {
                if let Some(class_name_node) = foreign_key_child.named_child(0)
                    && let Some(class_name) =
                        get_string_at_byte_range(content, class_name_node.byte_range())
                {
                    foreign_key.referenced_class = class_name;
                }
            }
            "index_name" => {
                if let Some(index_name_node) = foreign_key_child.named_child(0) {
                    foreign_key.referenced_index =
                        get_string_at_byte_range(content, index_name_node.byte_range());
                }
            }
            "foreignkey_keywords" => {
                let tracked_keywords = get_tracked_keywords(foreign_key_child, content);
                foreign_key.on_update = tracked_keywords.on_update;
                foreign_key.on_delete = tracked_keywords.on_delete;
            }
            _ => {
                eprintln!(
                    "Error: Unrecognized foreign key child node {:?}",
                    foreign_key_child.kind()
                );
                continue;
            }
        }
    }
    if &foreign_key.name == "TODO" || &foreign_key.referenced_class == "TODO" {
        eprintln!(
            "Error: failed to parse foreign key name {:?} or referenced class {:?}",
            &foreign_key.name, &foreign_key.referenced_class
        );
        return None;
    }
    Some(foreign_key)
}
