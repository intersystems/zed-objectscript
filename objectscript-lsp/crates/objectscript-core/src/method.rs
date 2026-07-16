use crate::common::{
    find_return_type, find_var_dependencies, generic_skipping_statements, get_keyword_and_value,
    get_node_children, get_string_at_byte_range,
};
use crate::parse_structures::{CodeMode, Language, Method, MethodType, ReturnType, Variable};

use std::collections::HashMap;
use tree_sitter::{Node, Range};

/// Builds a `Method` from its header/definition node (first-pass parse).
///
/// Parses the method name, return type, and method keywords (ProcedureBlock/Language/CodeMode,
/// visibility, and public variable list). Does **not** parse the method body statements; those
/// are handled in a later pass.
///
/// Returns the constructed `Method` and the source `Range` for the definition node.
pub fn initial_build_method(
    node: Node,
    method_type: MethodType,
    content: &str,
) -> Option<(Method, Range)> {
    let Some(method_name_node) = node.named_child(0) else {
        eprintln!(
            "Error: Expected method definition node to have child at index 0, aborting initial_build_method"
        );
        return None;
    };
    let Some(method_name) = get_string_at_byte_range(content, method_name_node.byte_range()) else {
        return None;
    };
    let method_range = node.range();
    let mut method_return_type = None;
    let mut is_procedure_block = None;
    let mut language = None;
    let mut codemode = None;
    let mut is_public = true;
    let mut public_variables = Vec::new();
    let children = get_node_children(node.clone());
    if children.len() <= 1 {
        eprintln!(
            "Error: Expected method definition node to have more than one child, aborting initial_build_method"
        );
        return None;
    }
    for node in children[1..].iter() {
        match node.kind() {
            "return_type" => {
                let Some(type_name_node) = node.named_child(1) else {
                    eprintln!(
                        "Warning: Expected node of kind ({:?}) to have a child at index 1, but it doesn't",
                        node.kind()
                    );
                    generic_skipping_statements("initial_build_method", node.kind(), "node");
                    continue;
                };
                let Some(typename) = get_string_at_byte_range(content, type_name_node.byte_range())
                else {
                    continue;
                };
                method_return_type = Some(find_return_type(typename));
            }
            "method_keyword"
            | "method_keyword_codemode_expression"
            | "call_method_keyword"
            | "method_keyword_external_language" => {
                let Some(keyword_str) = get_string_at_byte_range(content, node.byte_range()) else {
                    eprintln!("Error: Failed to get keyword string from byte range");
                    continue;
                };
                let (not, keyword_name, values) = get_keyword_and_value(keyword_str.as_str());
                if keyword_name == "procedureblock" {
                    if values.get(0).copied().is_none() {
                        is_procedure_block = Some(true);
                        continue;
                    }
                    let Some(value) = values.get(0).copied() else {
                        eprintln!("Error: Expected a value for procedureblock keyword, got: None");
                        continue;
                    };
                    if value == "1" {
                        is_procedure_block = Some(true);
                    } else if value == "0" {
                        is_procedure_block = Some(false);
                    } else {
                        eprintln!(
                            "Error: Expected procedureblock value to be '1' or '0', got: {}",
                            value
                        );
                        continue;
                    }
                } else if keyword_name == "language" {
                    let Some(value) = values.get(0).copied() else {
                        eprintln!("Error: Expected a value for language keyword, got: None");
                        continue;
                    };
                    if value == "objectscript" {
                        language = Some(Language::Objectscript);
                    } else if value == "tsql" {
                        language = Some(Language::TSql);
                    } else if value == "ispl" {
                        language = Some(Language::ISpl);
                    } else if value == "python" {
                        language = Some(Language::Python);
                    } else {
                        eprintln!(
                            "Error: Expected class keyword language to be 'objectscript' or 'tsql', got: {}",
                            value
                        );
                        continue;
                    }
                } else if keyword_name == "private" {
                    if not {
                        is_public = true;
                    } else {
                        is_public = false;
                    }
                } else if keyword_name == "codemode" {
                    let Some(value) = values.get(0).copied() else {
                        eprintln!("Expected a value for language keyword, got: None");
                        continue;
                    };
                    if value == "call" {
                        codemode = Some(CodeMode::Call);
                    } else if value == "code" {
                        codemode = Some(CodeMode::Code);
                    } else if value == "expression" {
                        codemode = Some(CodeMode::Expression);
                    } else if value == "objectgenerator" {
                        codemode = Some(CodeMode::ObjectGenerator);
                    } else {
                        eprintln!(
                            "Expected class keyword codemode to be 'call', 'code', 'expression', or 'objectgenerator', got: {}",
                            value
                        );
                        continue;
                    }
                } else if keyword_name == "publiclist" {
                    for variable in values {
                        public_variables.push(variable.to_string());
                    }
                }
            }
            _ => {
                // only parse the header for initial build
                continue;
            }
        }
    }
    let method = Method::new(
        method_name.clone(),
        is_procedure_block,
        language,
        codemode.unwrap_or(CodeMode::Code),
        is_public,
        method_return_type,
        public_variables,
        method_type,
    );
    Some((method, method_range))
}

impl Method {
    /// Creates a new `Method` from parsed header information.
    ///
    /// Initializes empty variable tables and stores declared keywords/visibility/type metadata.
    pub fn new(
        method_name: String,
        is_procedure_block: Option<bool>,
        language: Option<Language>,
        code_mode: CodeMode,
        is_public: bool,
        return_type: Option<ReturnType>,
        public_variables: Vec<String>,
        method_type: MethodType,
    ) -> Self {
        Self {
            method_type,
            return_type,
            name: method_name,
            variables: HashMap::new(),
            is_public,
            is_procedure_block,
            language,
            code_mode,
            public_variables_declared: public_variables,
        }
    }

    /// Parses a method definition node to extract variables and their dependencies.
    ///
    /// Collects:
    /// - argument variables from the `arguments` node
    /// - variables assigned via `set` statements in the core body
    ///
    /// Returns a list of `(variable, definition_range, var_dependencies)`.
    /// Visibility (public vs private) is inferred from ProcedureBlock and `public_variables_declared`.
    pub fn build_method_variable_defs(
        &self,
        node: Node,
        content: &str,
    ) -> Vec<(Variable, Range, Vec<String>)> {
        let mut variables: Vec<(Variable, Range, Vec<String>)> = Vec::new();
        let children = get_node_children(node.clone());
        for node in children.iter().skip(1) {
            if node.kind() == "arguments" {
                let children = get_node_children(node.clone());
                for node in children {
                    // each node is an argument
                    let argument_children = get_node_children(node);

                    let (variable_name, var_name_range, arg_type) = {
                        let mut name = None;
                        let mut var_range = None;
                        let mut arg_type = None;
                        for arg_child in argument_children {
                            if arg_child.kind() == "method_arg" {
                                if let Some(method_arg_type) = arg_child.named_child(0) {
                                    let Some(variable_name_node) = method_arg_type.named_child(0)
                                    else {
                                        eprintln!(
                                            "Error: Expression,byref_arg, and variadic_arg nodes all have a child at index 0, but this does not {:?}",
                                            method_arg_type.kind()
                                        );
                                        break;
                                    };
                                    name = get_string_at_byte_range(
                                        content,
                                        variable_name_node.byte_range(),
                                    );
                                    var_range = Some(variable_name_node.range());
                                } else {
                                    eprintln!(
                                        "Error: Method arg node should have named children. This node didn't {:?}",
                                        arg_child.kind()
                                    );
                                    break;
                                }
                            } else if arg_child.kind() == "return_type" {
                                let Some(typename) = arg_child.named_child(1) else {
                                    eprintln!(
                                        "Error: expected return_type node to have typename at index 1"
                                    );
                                    continue;
                                };
                                if let Some(return_type_str) =
                                    get_string_at_byte_range(content, typename.byte_range())
                                    && typename.kind() == "typename"
                                {
                                    arg_type = Some(find_return_type(return_type_str));
                                } else {
                                    eprintln!(
                                        "Error: expected return_type node to have typename at index 1"
                                    );
                                    continue;
                                }
                            }
                        }
                        (name, var_range, arg_type)
                    };
                    let Some(var_name) = variable_name else {
                        continue;
                    };
                    let Some(var_name_range) = var_name_range else {
                        continue;
                    };
                    if self.is_procedure_block.unwrap_or(true) == false
                        || self.public_variables_declared.contains(&var_name)
                    {
                        let var = Variable::new(var_name, arg_type, true, false, None);
                        variables.push((var, var_name_range, Vec::new()));
                    } else {
                        let var = Variable::new(var_name, arg_type, false, false, None);
                        variables.push((var, var_name_range, Vec::new()));
                    }
                }
            } else if node.kind() == "statement" {
                let Some(command) = node.named_child(0) else {
                    eprintln!("Error: failed to get child node (index 0) from statement node");
                    continue;
                };
                match command.kind() {
                    "command_set" => {
                        let Some(set_argument) = command.named_child(1) else {
                            eprintln!(
                                "Error: failed to get child node (index 1) from command_set node"
                            );
                            continue;
                        };
                        let mut var_defs = Vec::new();
                        let set_argument_children = get_node_children(set_argument);
                        for set_arg_child in set_argument_children {
                            match set_arg_child.kind() {
                                "set_target" => {
                                    let Some(set_target_child) = set_arg_child.named_child(0)
                                    else {
                                        eprintln!(
                                            "Error: Expected child at index 0 for set_target node {:?}",
                                            set_arg_child.kind()
                                        );
                                        continue;
                                    };
                                    let var_range = set_target_child.range();
                                    match set_target_child.kind() {
                                        "gvn" => {
                                            let gvn_children = get_node_children(set_target_child);
                                            for gvn_child in gvn_children {
                                                if gvn_child.kind() == "identifier" {
                                                    if let Some(gvn_id) = get_string_at_byte_range(
                                                        content,
                                                        gvn_child.byte_range(),
                                                    ) {
                                                        var_defs.push((gvn_id, var_range));
                                                    }
                                                }
                                            }
                                        }
                                        "lvn" => {
                                            let Some(lvn_id_node) = set_target_child.named_child(0)
                                            else {
                                                eprintln!(
                                                    "Parsing Error: lvn must have a child at index 0, update parsing"
                                                );
                                                continue;
                                            };
                                            if let Some(lvn_id) = get_string_at_byte_range(
                                                content,
                                                lvn_id_node.byte_range(),
                                            ) {
                                                var_defs.push((lvn_id, var_range));
                                            }
                                        }
                                        _ => {
                                            eprintln!(
                                                "Warning: set target case: {:?} not yet implemented, skipping.",
                                                set_target_child.kind()
                                            );
                                        }
                                    }
                                }
                                "expression" => {
                                    let mut var_deps = Vec::new();
                                    let (is_oref, curr_class) = find_var_dependencies(
                                        set_arg_child,
                                        content,
                                        &mut var_deps,
                                    );
                                    for (var_name, v_range) in &var_defs {
                                        let is_public = self.is_procedure_block.unwrap_or(true)
                                            == false
                                            || self.public_variables_declared.contains(var_name);
                                        let variable = Variable::new(
                                            var_name.clone(),
                                            None,
                                            is_public,
                                            is_oref,
                                            curr_class.clone(),
                                        );
                                        variables.push((variable, *v_range, var_deps.clone()));
                                    }
                                }
                                _ => continue,
                            }
                        }
                    }
                    _ => {
                        continue;
                    }
                }
            }
        }
        variables
    }

    /// Applies inherited class keywords to this method when not explicitly set.
    ///
    /// - Inherits `ProcedureBlock=false` only when the method has no explicit setting.
    /// - Inherits the class `default_language` when the method language is unset.
    pub fn update_keywords(&mut self, is_procedure_block: bool, default_language: Language) {
        // inherit class keywords if not specified and class keyword isn't the default value
        if self.is_procedure_block.is_none() && is_procedure_block == false {
            // inherit the class keyword when it isn't the default
            self.is_procedure_block = Some(is_procedure_block);
        }

        if self.language.is_none() {
            // inherit the class keyword when it isn't the default
            self.language = Some(default_language.clone());
        }
    }
}
