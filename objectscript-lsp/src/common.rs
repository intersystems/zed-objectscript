use objectscript_core::common::get_node_children;
use tree_sitter::Node;
/// Generate a human-readable diagnostic message for a syntax error node.
pub fn diagnostic_message(node: Node, error_text: &str) -> Option<String> {
    if let Some(sibling_node) = node.prev_named_sibling() {
        match sibling_node.kind() {
            "statement" => {
                let child = sibling_node.named_child(0);
                if let Some(child) = child {
                    match child.kind() {
                        "command_set" => {
                            let children = get_node_children(child);
                            if let Some(last_child) = children.last() {
                                match last_child.kind() {
                                    "keyword_set" => {
                                        let Some(_) = node.parent() else {
                                            return Some(format!(
                                                "Syntax Error: Invalid set command {}",
                                                error_text
                                            ));
                                        };
                                        return Some(format!(
                                            "Syntax Error: Expected a variable name, got {}",
                                            error_text
                                        ));
                                    }
                                    "set_argument" => {
                                        let set_arg_children =
                                            get_node_children(last_child.clone());
                                        if let Some(child) = set_arg_children.last() {
                                            match child.kind() {
                                                "set_target" | "set_target_list" => {
                                                    if let Some(next_sib) = child.next_sibling() {
                                                        if next_sib.kind() == "=" {
                                                            return Some(format!(
                                                                "Syntax Error: Expected an expression, {} is not a valid expression.",
                                                                error_text
                                                            ));
                                                        }
                                                    };
                                                    return Some(format!(
                                                        "Syntax Error: Expected '=' or another variable name separated with by a comma and contained within parenthesis, got {}",
                                                        error_text
                                                    ));
                                                }
                                                "expression" => {
                                                    return Some(format!(
                                                        "Syntax Error: Unexpected, {} after an expression. Expected a binary operator or end of SET command",
                                                        error_text
                                                    ));
                                                }

                                                _ => return None,
                                            }
                                        }
                                        return None;
                                    }
                                    _ => {
                                        return None;
                                    }
                                }
                            }
                        }
                        _ => {
                            return None;
                        }
                    }
                }
            }
            _ => {
                return None;
            }
        }
    }
    None
}
