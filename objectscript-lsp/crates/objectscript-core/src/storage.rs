use crate::common::get_string_at_byte_range;
use crate::parse_structures::Storage;
use tree_sitter::Node;
pub fn build_storage_struct(storage_node: Node, content: &str) -> Option<Storage> {
    if storage_node.kind() != "storage" {
        eprintln!(
            "Error: build_storage_struct was called for node {:?}, but it can only be called for storage nodes",
            storage_node.kind()
        );
        return None;
    }
    if let Some(storage_name_outer_node) = storage_node.named_child(1)
        && storage_name_outer_node.kind() == "storage_name"
        && let Some(storage_name_node) = storage_name_outer_node.named_child(0)
        && let Some(name) = get_string_at_byte_range(content, storage_name_node.byte_range())
    {
        return Some(Storage { name });
    }
    None
}
