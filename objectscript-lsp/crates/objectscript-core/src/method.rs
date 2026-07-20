use crate::common::{
    find_return_type, find_var_dependencies, generic_skipping_statements, get_keyword_and_value,
    get_node_children, get_string_at_byte_range,
};
use crate::parse_structures::{CodeMode, Language, Method, MethodType, ReturnType, Variable};

use std::collections::HashMap;
use tree_sitter::{Language as TsLanguage, Node, Query, QueryCursor, Range, StreamingIterator};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;

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

    pub fn build_variables(
        &self,
        node: Node,
        content: &str,
        is_rtn: bool,
    ) -> Vec<(Variable, Range, Vec<String>)> {
        let mut variables = Vec::new();
        let language: TsLanguage;
        if is_rtn {
            language = LANGUAGE_OBJECTSCRIPT_ROUTINE.into()
        } else {
            language = LANGUAGE_OBJECTSCRIPT_UDL.into();
        }
        let query_str = "(argument (method_arg) @arg (return_type (typename) @return)?)";
        variables.extend(self.get_method_arguments(&language, query_str, node, content));

        let query_str = "(command_set (set_argument [(set_target) (set_target_list)] @settarget (expression) @value ))";
        variables.extend(self.get_set_command_variable_defs(&language, query_str, node, content));
        variables
    }

    fn get_set_command_variable_defs(
        &self,
        language: &TsLanguage,
        query_str: &str,
        node: Node,
        content: &str,
    ) -> Vec<(Variable, Range, Vec<String>)> {
        let mut variables = Vec::new();
        if let Ok(query) = Query::new(language, query_str) {
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(&query, node, content.as_bytes());
            while let Some(query_match) = iter.next() {
                let mut var_defs = Vec::new();
                let mut var_deps = Vec::new();
                let set_target_node = query_match.captures[0].node;
                let var_value = query_match.captures[1].node;
                let children;
                if set_target_node.kind() == "set_target_list" {
                    children = get_node_children(set_target_node);
                } else {
                    children = vec![set_target_node];
                }
                for set_target in children {
                    let Some(set_target_child) = set_target.named_child(0) else {
                        eprintln!(
                            "Error: Expected child at index 0 for set_target node {:?}",
                            set_target.kind()
                        );
                        continue;
                    };
                    let var_range = set_target_child.range();
                    match set_target_child.kind() {
                        "gvn" => {
                            let gvn_children = get_node_children(set_target_child);
                            for gvn_child in gvn_children {
                                if gvn_child.kind() == "identifier" {
                                    if let Some(gvn_id) =
                                        get_string_at_byte_range(content, gvn_child.byte_range())
                                    {
                                        var_defs.push((gvn_id, var_range));
                                    }
                                }
                            }
                        }
                        "lvn" => {
                            let Some(lvn_id_node) = set_target_child.named_child(0) else {
                                eprintln!(
                                    "Parsing Error: lvn must have a child at index 0, update parsing"
                                );
                                continue;
                            };
                            if let Some(lvn_id) =
                                get_string_at_byte_range(content, lvn_id_node.byte_range())
                            {
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
                let (is_oref, curr_class) =
                    find_var_dependencies(var_value, content, &mut var_deps);
                for (var_name, v_range) in &var_defs {
                    let is_public = self.is_procedure_block.unwrap_or(true) == false
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
        }
        variables
    }

    fn get_method_arguments(
        &self,
        language: &TsLanguage,
        query_str: &str,
        node: Node,
        content: &str,
    ) -> Vec<(Variable, Range, Vec<String>)> {
        let mut variables = Vec::new();
        if let Ok(query) = Query::new(language, query_str) {
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(&query, node, content.as_bytes());
            while let Some(query_match) = iter.next() {
                let name;
                let var_range;
                let mut arg_type = None;
                let method_arg;
                let mut return_type = None;
                method_arg = query_match.captures[0].node;
                if query_match.captures.len() > 1 {
                    return_type = Some(query_match.captures[1].node);
                }
                if let Some(method_arg_type) = method_arg.named_child(0) {
                    let Some(variable_name_node) = method_arg_type.named_child(0) else {
                        eprintln!(
                            "Error: Expression,byref_arg, and variadic_arg nodes all have a child at index 0, but this does not {:?}",
                            method_arg_type.kind()
                        );
                        break;
                    };
                    name = get_string_at_byte_range(content, variable_name_node.byte_range());
                    var_range = Some(variable_name_node.range());
                } else {
                    eprintln!(
                        "Error: Method arg node should have named children. This node didn't {:?}",
                        method_arg.kind()
                    );
                    continue;
                }
                if let Some(return_type) = return_type
                    && let Some(return_type_str) =
                        get_string_at_byte_range(content, return_type.byte_range())
                    && return_type.kind() == "typename"
                {
                    arg_type = Some(find_return_type(return_type_str));
                }
                if let Some(var_name) = name
                    && let Some(var_name_range) = var_range
                {
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
