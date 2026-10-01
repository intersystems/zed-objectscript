use crate::common::{
    get_node_children, get_string_at_byte_range, get_tracked_keywords, parse_return_type,
};
use crate::parse_structures::Parameter;
use tree_sitter::Node;

impl Default for Parameter {
    fn default() -> Self {
        Self {
            is_final: None,
            name: "TODO".to_string(),
            return_type: None,
            default_value: None,
        }
    }
}

pub fn build_parameter_struct(parameter_node: Node, content: &str) -> Option<Parameter> {
    if parameter_node.kind() != "parameter" {
        eprintln!(
            "Error: build_parameter_struct was called for node {:?}, but it can only be called for parameter nodes",
            parameter_node.kind()
        );
        return None;
    }
    let mut parameter = Parameter::default();
    let parameter_children = get_node_children(parameter_node);
    for parameter_child in parameter_children {
        match parameter_child.kind() {
            "parameter_name" => {
                if let Some(parameter_name_node) = parameter_child.named_child(0)
                    && let Some(name) =
                        get_string_at_byte_range(content, parameter_name_node.byte_range())
                {
                    parameter.name = name;
                } else {
                    eprintln!("Error: failed to get parameter name.");
                    return None;
                }
            }
            "default_argument_value" => {
                if let Some(val) = parameter_child.named_child(0) {
                    parameter.default_value = get_string_at_byte_range(content, val.byte_range());
                }
            }
            "return_type" => {
                parameter.return_type = parse_return_type(parameter_child, content);
            }
            "parameter_keywords" => {
                let tracked_keywords = get_tracked_keywords(parameter_child, content);
                parameter.is_final = tracked_keywords.is_final;
            }
            _ => continue,
        }
    }
    if &parameter.name == "TODO" {
        eprintln!("error: failed to parse parameter name");
        return None;
    }
    Some(parameter)
}
