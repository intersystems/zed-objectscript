use crate::common::{get_node_children, get_string_at_byte_range, get_tracked_keywords};
use crate::parse_structures::{Language, XData};
use tree_sitter::Node;

impl Default for XData {
    fn default() -> Self {
        Self {
            name: "TODO".to_string(),
            language: Language::Xml,
        }
    }
}

pub fn build_xdata_struct(xdata_node: Node, content: &str) -> Option<XData> {
    if xdata_node.kind() != "xdata" {
        eprintln!(
            "Error: build_xdata_struct was called for node {:?}, but it can only be called for xdata nodes",
            xdata_node.kind()
        );
        return None;
    }
    let mut xdata = XData::default();
    let xdata_children = get_node_children(xdata_node);
    for xdata_child in xdata_children {
        match xdata_child.kind() {
            "xdata_name" => {
                if let Some(xdata_name_node) = xdata_child.named_child(0)
                    && let Some(name) =
                        get_string_at_byte_range(content, xdata_name_node.byte_range())
                {
                    xdata.name = name;
                } else {
                    eprintln!("Error: failed to get xdata name.");
                    return None;
                }
            }
            "xdata_keywords" => {
                let tracked_keywords = get_tracked_keywords(xdata_child, content);
                xdata.language = tracked_keywords.language.unwrap_or(Language::Xml);
            }
            _ => {
                continue;
            }
        }
    }
    if &xdata.name == "TODO" {
        eprintln!("error: failed to parse xdata name {:?}", &xdata.name);
        return None;
    }
    Some(xdata)
}
