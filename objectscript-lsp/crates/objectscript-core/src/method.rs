use crate::common::{
    find_return_type, find_var_dependencies, get_keyword_and_value, get_node_children,
    get_string_at_byte_range, parse_line_ref, range_within_range,
};
use crate::parse_structures::{
    CodeMode, Language, Method, MethodType, TypeName, UnresolvedMethodRef, Variable,
};

use crate::scope_structures::ScopeId;
use crate::scope_tree::ScopeTree;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use tree_sitter::{Language as TsLanguage, Node, Query, QueryCursor, Range, StreamingIterator};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;

const SET_VARIABLES_QUERY: &str =
    "(command_set (set_argument [(set_target) (set_target_list)] @settarget (expression) @value ))";

const ROUTINE_ARGUMENT_QUERY: &str = "(tag_parameter (method_arg) @arg)";

const CLASS_METHOD_ARGUMENT_QUERY: &str =
    "(argument (method_arg) @arg (return_type (typename) @typename)?)";

const METHOD_DEPENDENCY_QUERY: &str = r#"[(class_method_call) @classmethodcall
(system_defined_function) @systemfunc
(relative_dot_method) @relativemethod
(routine_tag_call) @routine
(goto_argument) @routine
(print_argument) @routine
]"#;

const METHOD_KEYWORD_QUERY: &str = r#"
    (method_definition ([(method_keyword_codemode_expression) @keyword
                (method_keyword_external_language) @keyword
                (method_keyword) @keyword
                (call_method_keyword) @keyword
                (return_type (typename (identifier) @returntype ))
                ]))"#;

fn cached_query(
    query: &'static OnceLock<Query>,
    language: TsLanguage,
    source: &str,
    name: &str,
) -> &'static Query {
    query.get_or_init(|| {
        Query::new(&language, source)
            .unwrap_or_else(|error| panic!("failed to compile {name} Tree-sitter query: {error}"))
    })
}

fn udl_set_variables_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_UDL.into(),
        SET_VARIABLES_QUERY,
        "UDL set variables",
    )
}

fn routine_set_variables_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_ROUTINE.into(),
        SET_VARIABLES_QUERY,
        "routine set variables",
    )
}

fn routine_argument_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_ROUTINE.into(),
        ROUTINE_ARGUMENT_QUERY,
        "routine argument",
    )
}

fn class_method_argument_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_UDL.into(),
        CLASS_METHOD_ARGUMENT_QUERY,
        "class method argument",
    )
}

fn udl_method_dependency_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_UDL.into(),
        METHOD_DEPENDENCY_QUERY,
        "UDL method dependency",
    )
}

fn routine_method_dependency_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_ROUTINE.into(),
        METHOD_DEPENDENCY_QUERY,
        "routine method dependency",
    )
}

fn method_keyword_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_UDL.into(),
        METHOD_KEYWORD_QUERY,
        "method keyword",
    )
}

impl Method {
    /// Creates a new `Method` from parsed header information.
    ///
    /// Initializes empty variable tables and stores declared keywords/visibility/type metadata.
    pub fn new(
        method_name: String,
        public_variables: HashSet<String>,
        method_type: MethodType,
    ) -> Self {
        return match method_type {
            MethodType::Routine => Self {
                method_type,
                return_type: None,
                name: method_name,
                variables: HashMap::new(),
                is_public: true,
                is_procedure_block: Some(false),
                language: None,
                public_variables_declared: public_variables,
                code_mode: CodeMode::Code,
                is_final: Some(true),
            },
            MethodType::Subroutine(is_public) | MethodType::DottedSubroutine(is_public) => Self {
                method_type,
                return_type: None,
                name: method_name,
                variables: HashMap::new(),
                is_public: is_public,
                is_procedure_block: Some(false),
                language: None,
                public_variables_declared: public_variables,
                code_mode: CodeMode::Code,
                is_final: Some(true),
            },
            MethodType::Procedure(is_public) => Self {
                method_type,
                return_type: None,
                name: method_name,
                variables: HashMap::new(),
                is_public: is_public,
                is_procedure_block: Some(true),
                language: None,
                public_variables_declared: public_variables,
                code_mode: CodeMode::Code,
                is_final: Some(true),
            },
            MethodType::ClassMethod | MethodType::InstanceMethod => Self {
                method_type,
                return_type: None,
                name: method_name,
                variables: HashMap::new(),
                is_public: true,
                is_procedure_block: None,
                language: None,
                public_variables_declared: public_variables,
                code_mode: CodeMode::Code,
                is_final: None,
            },
        };
    }

    fn build_subroutine_set_variables(
        &self,
        node: Node,
        content: &str,
        scope_tree: &ScopeTree,
        variables_in_method: &mut Vec<(Variable, Range, Vec<String>, ScopeId)>,
        method_range: Range,
    ) {
        {
            let query = routine_set_variables_query();
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(query, node, content.as_bytes());
            while let Some(query_match) = iter.next() {
                let set_target_node = query_match.captures[0].node;
                if !range_within_range(&set_target_node.range(), &method_range) {
                    continue;
                }
                let mut var_defs = Vec::new();
                let mut var_deps = Vec::new();
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
                for (variable_name, var_range) in &var_defs {
                    let var = Variable::new(
                        variable_name.clone(),
                        None,
                        true,
                        is_oref,
                        curr_class.clone(),
                    );
                    if let Some(scope_id) = scope_tree
                        .find_current_scope_for_range(var_range.start_point, var_range.end_point)
                    {
                        variables_in_method.push((var, *var_range, var_deps.clone(), scope_id));
                    }
                }
            }
        }
    }

    fn build_procedure_set_variables(
        &self,
        node: Node,
        content: &str,
        scope_tree: &ScopeTree,
        variables_in_method: &mut Vec<(Variable, Range, Vec<String>, ScopeId)>,
        class_is_procedure_block: Option<bool>,
        is_class_method: bool,
    ) {
        {
            let query = if is_class_method {
                udl_set_variables_query()
            } else {
                routine_set_variables_query()
            };
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(query, node, content.as_bytes());
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
                for (variable_name, var_range) in &var_defs {
                    let variable_is_public = if !is_class_method {
                        if self.public_variables_declared.contains(variable_name) {
                            true
                        } else {
                            false
                        }
                    } else {
                        self.is_procedure_block
                            .unwrap_or(class_is_procedure_block.unwrap_or(true))
                            == false
                            || self.public_variables_declared.contains(variable_name)
                    };
                    let var = Variable::new(
                        variable_name.clone(),
                        None,
                        variable_is_public,
                        is_oref,
                        curr_class.clone(),
                    );
                    if let Some(scope_id) = scope_tree
                        .find_current_scope_for_range(var_range.start_point, var_range.end_point)
                    {
                        variables_in_method.push((var, *var_range, var_deps.clone(), scope_id));
                    }
                }
            }
        }
    }

    /// Given tag node, parse the arguments
    fn build_routine_method_arguments(
        &self,
        tag_node: Node,
        content: &str,
        scope_tree: &ScopeTree,
        variables_in_method: &mut Vec<(Variable, Range, Vec<String>, ScopeId)>,
        is_procedure: bool, // false if subroutine
    ) {
        {
            let query = routine_argument_query();
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(query, tag_node, content.as_bytes());
            while let Some(query_match) = iter.next() {
                let method_arg = query_match.captures[0].node;
                if let Some(method_arg_type) = method_arg.named_child(0) {
                    let Some(variable_name_node) = method_arg_type.named_child(0) else {
                        eprintln!(
                            "Error: Expression, byref_arg, and variadic_arg nodes all have a child at index 0, but this does not {:?}",
                            method_arg_type.kind()
                        );
                        break;
                    };
                    if let Some(var_name) =
                        get_string_at_byte_range(content, variable_name_node.byte_range())
                    {
                        let var_range = variable_name_node.range();
                        let variable_is_public = if !is_procedure
                            || self.public_variables_declared.contains(&var_name)
                        {
                            true
                        } else {
                            false
                        };
                        let var = Variable::new(var_name, None, variable_is_public, false, None);
                        if let Some(scope_id) = scope_tree.find_current_scope_for_range(
                            var_range.start_point,
                            var_range.end_point,
                        ) {
                            variables_in_method.push((var, var_range, Vec::new(), scope_id));
                        }
                    }
                } else {
                    eprintln!(
                        "Error: Method arg node should have named children. This node didn't {:?}",
                        method_arg.kind()
                    );
                }
                continue;
            }
        }
    }

    fn build_class_method_arguments(
        &self,
        node: Node,
        content: &str,
        scope_tree: &ScopeTree,
        variables_in_method: &mut Vec<(Variable, Range, Vec<String>, ScopeId)>,
        class_is_procedure_block: Option<bool>,
    ) {
        {
            let query = class_method_argument_query();
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(query, node, content.as_bytes());
            let arg_idx = query.capture_index_for_name("arg");
            let typename_idx = query.capture_index_for_name("typename");
            let mut return_type_parameters = Vec::new();
            let mut return_type_id = None;
            let mut var_range = None;
            let mut var_name = None;
            while let Some(query_match) = iter.next() {
                let mut i = 0;
                let mut arg_type = None;
                while i < query_match.captures.len() {
                    let capture = &query_match.captures[i];
                    if arg_idx == Some(capture.index) {
                        let method_arg = capture.node;
                        if let Some(method_arg_type) = method_arg.named_child(0) {
                            let Some(variable_name_node) = method_arg_type.named_child(0) else {
                                eprintln!(
                                    "Error: Expression,byref_arg, and variadic_arg nodes all have a child at index 0, but this does not {:?}",
                                    method_arg_type.kind()
                                );
                                break;
                            };
                            var_name =
                                get_string_at_byte_range(content, variable_name_node.byte_range());
                            var_range = Some(variable_name_node.range());
                        } else {
                            eprintln!(
                                "Error: Method arg node should have named children. This node didn't {:?}",
                                method_arg.kind()
                            );
                        }
                        i += 1;
                        continue;
                    } else if typename_idx == Some(capture.index) {
                        let typename = capture.node;
                        let identifiers = get_node_children(typename);
                        let mut j = 0;
                        while j < identifiers.len() {
                            let identifier_node = &identifiers[j];
                            let Some(typename_identifier) =
                                get_string_at_byte_range(content, identifier_node.byte_range())
                            else {
                                j += 1;
                                continue;
                            };
                            if j == 0 {
                                return_type_id = Some(find_return_type(typename_identifier));
                            } else {
                                return_type_parameters.push(typename_identifier);
                            }
                            j += 1;
                            continue;
                        }
                        if let Some(typename_id) = &return_type_id {
                            arg_type = Some(TypeName {
                                ret_type: typename_id.clone(),
                                parameters: return_type_parameters.clone(),
                            })
                        }

                        i += 1;
                        continue;
                    }
                    i += 1;
                }
                if let Some(var_name) = &var_name
                    && let Some(var_range) = var_range
                {
                    let variable_is_public = self
                        .is_procedure_block
                        .unwrap_or(class_is_procedure_block.unwrap_or(true))
                        == false
                        || self.public_variables_declared.contains(var_name);
                    let var =
                        Variable::new(var_name.clone(), arg_type, variable_is_public, false, None);
                    if let Some(scope_id) = scope_tree
                        .find_current_scope_for_range(var_range.start_point, var_range.end_point)
                    {
                        variables_in_method.push((var, var_range, Vec::new(), scope_id));
                    }
                }
            }
        }
    }

    fn get_method_dependencies(
        &mut self,
        node: Node,
        content: &str,
        is_class_method: bool,
        class_name: &str,
        method_range: Range,
    ) -> (
        HashSet<UnresolvedMethodRef>,
        HashSet<(String, String, Range, String)>,
    ) {
        let mut unresolved_method_refs = HashSet::new();
        let mut unresolved_oref_method_refs = HashSet::new();
        {
            let query = if is_class_method {
                udl_method_dependency_query()
            } else {
                routine_method_dependency_query()
            };
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(query, node, content.as_bytes());
            let classmethod_idx = query.capture_index_for_name("classmethodcall");
            let systemfunc_idx = query.capture_index_for_name("systemfunc");
            let relativemethod_idx = query.capture_index_for_name("relativemethod");
            let routine_idx = query.capture_index_for_name("routine");
            while let Some(query_match) = iter.next() {
                let mut i = 0;
                while i < query_match.captures.len() {
                    let capture = &query_match.captures[i];
                    let matched_node = capture.node;
                    if !range_within_range(&matched_node.range(), &method_range) {
                        i += 1;
                        continue;
                    }
                    if classmethod_idx == Some(capture.index) {
                        if let Some(class_ref) = matched_node.named_child(0)
                            && let Some(method_name_outer) = matched_node.named_child(1)
                            && let Some(class_name_outer) = class_ref.named_child(1)
                            && let Some(method_name_node) = method_name_outer.named_child(0)
                            && let Some(class_name_node) = class_name_outer.named_child(0)
                            && let Some(method_name) =
                                get_string_at_byte_range(content, method_name_node.byte_range())
                            && let Some(class_name) =
                                get_string_at_byte_range(content, class_name_node.byte_range())
                        {
                            unresolved_method_refs.insert(UnresolvedMethodRef {
                                class: class_name,
                                method: method_name,
                                offset: None,
                                method_call_range: matched_node.range(),
                            });
                        }
                    } else if systemfunc_idx == Some(capture.index) {
                        let Some(node_str) =
                            get_string_at_byte_range(content, matched_node.byte_range())
                        else {
                            i += 1;
                            continue;
                        };
                        let (before, method_args) = (
                            node_str.split('(').nth(0),
                            node_str.split('(').nth(1).unwrap_or(""),
                        );
                        if let Some(func_name) = before {
                            if func_name.eq_ignore_ascii_case("$zobjmethod")
                                || func_name.eq_ignore_ascii_case("$method")
                            {
                                if let Some(oref_method_arg) = matched_node.named_child(0)
                                    && let Some(oref_method_arg_type) =
                                        oref_method_arg.named_child(0)
                                    && let Some(oref_name_node) =
                                        oref_method_arg_type.named_child(0)
                                    && let Some(oref_var_name) = get_string_at_byte_range(
                                        content,
                                        oref_name_node.byte_range(),
                                    )
                                    && let Some(method_name_method_arg) =
                                        matched_node.named_child(1)
                                    && let Some(method_name_arg_type) =
                                        method_name_method_arg.named_child(0)
                                    && let Some(method_name_node) =
                                        method_name_arg_type.named_child(0)
                                    && let Some(method_name) = get_string_at_byte_range(
                                        content,
                                        method_name_node.byte_range(),
                                    )
                                {
                                    unresolved_oref_method_refs.insert((
                                        oref_var_name,
                                        method_name,
                                        matched_node.range(),
                                        self.name.clone(),
                                    ));
                                }
                            } else if func_name.eq_ignore_ascii_case("$classmethod")
                                || func_name.eq_ignore_ascii_case("$zobjclassmethod")
                            {
                                if method_args.trim_start().chars().next() == Some(',') {
                                    // class is current one
                                    if let Some(method_name_method_arg) =
                                        matched_node.named_child(0)
                                        && let Some(method_name_arg_type) =
                                            method_name_method_arg.named_child(0)
                                        && let Some(method_name_node) =
                                            method_name_arg_type.named_child(0)
                                        && let Some(method_name) = get_string_at_byte_range(
                                            content,
                                            method_name_node.byte_range(),
                                        )
                                    {
                                        unresolved_method_refs.insert(UnresolvedMethodRef {
                                            class: class_name.to_string(),
                                            method: method_name,
                                            offset: None,
                                            method_call_range: matched_node.range(),
                                        });
                                    }
                                } else {
                                    if let Some(classname_method_arg) = matched_node.named_child(0)
                                        && let Some(classname_method_arg_type) =
                                            classname_method_arg.named_child(0)
                                        && let Some(classname_node) =
                                            classname_method_arg_type.named_child(0)
                                        && let Some(classname_var) = get_string_at_byte_range(
                                            content,
                                            classname_node.byte_range(),
                                        )
                                        && let Some(method_name_method_arg) =
                                            matched_node.named_child(1)
                                        && let Some(method_name_arg_type) =
                                            method_name_method_arg.named_child(0)
                                        && let Some(method_name_node) =
                                            method_name_arg_type.named_child(0)
                                        && let Some(method_name) = get_string_at_byte_range(
                                            content,
                                            method_name_node.byte_range(),
                                        )
                                    {
                                        unresolved_method_refs.insert(UnresolvedMethodRef {
                                            class: classname_var,
                                            method: method_name,
                                            offset: None,
                                            method_call_range: matched_node.range(),
                                        });
                                    }
                                }
                            } else if func_name.eq_ignore_ascii_case("$system") {
                                if let Some(class_name_node) = matched_node.named_child(0)
                                    && let Some(method_name_node) = matched_node.named_child(1)
                                    && let Some(classname) = get_string_at_byte_range(
                                        content,
                                        class_name_node.byte_range(),
                                    )
                                    && let Some(method_name) = get_string_at_byte_range(
                                        content,
                                        method_name_node.byte_range(),
                                    )
                                {
                                    unresolved_method_refs.insert(UnresolvedMethodRef {
                                        class: classname,
                                        method: method_name,
                                        offset: None,
                                        method_call_range: matched_node.range(),
                                    });
                                }
                            }
                        }
                    } else if relativemethod_idx == Some(capture.index) {
                        if let Some(oref_method) = matched_node.named_child(0)
                            && let Some(method_name_node) = oref_method.named_child(0)
                            && let Some(method_identifier) = method_name_node.named_child(0)
                            && let Some(method_name) =
                                get_string_at_byte_range(content, method_identifier.byte_range())
                        {
                            unresolved_method_refs.insert(UnresolvedMethodRef {
                                class: class_name.to_string(),
                                method: method_name,
                                offset: None,
                                method_call_range: matched_node.range(),
                            });
                        }
                    } else if routine_idx == Some(capture.index) {
                        if let Some(routine_tag_call_child) = matched_node.named_child(0) {
                            match routine_tag_call_child.kind() {
                                "method_name" => {
                                    // this version doesn't have wrapped in quotes option
                                    if let Some(method_name) =
                                        get_string_at_byte_range(content, matched_node.byte_range())
                                    {
                                        unresolved_method_refs.insert(UnresolvedMethodRef {
                                            class: class_name.to_string(),
                                            method: method_name,
                                            offset: None,
                                            method_call_range: matched_node.range(),
                                        });
                                    }
                                }
                                "line_ref" => {
                                    let (routine_name, method_name, offset) = parse_line_ref(
                                        routine_tag_call_child,
                                        content,
                                        class_name.to_string(),
                                    );

                                    unresolved_method_refs.insert(UnresolvedMethodRef {
                                        class: routine_name,
                                        method: method_name,
                                        offset,
                                        method_call_range: matched_node.range(),
                                    });
                                }
                                _ => {
                                    i += 1;
                                    continue;
                                }
                            }
                        }
                    }
                    i += 1;
                    continue;
                }
            }
        }
        (unresolved_method_refs, unresolved_oref_method_refs)
    }

    /// Build Method Keywords and Body
    pub fn rebuild_method(
        &mut self,
        node: Node,
        content: &str,
        scope_tree: &ScopeTree,
        method_type: MethodType,
        method_range: Range,
        public_variables_declared: HashSet<String>, // only procedure passes this
        class_is_final: Option<bool>,
        old_class_is_final: Option<bool>,
        class_is_procedure_block: Option<bool>,
        class_name: &str,
    ) -> (
        bool,
        bool,
        Vec<(Variable, Range, Vec<String>, ScopeId)>,
        HashSet<UnresolvedMethodRef>,
        HashSet<(String, String, Range, String)>,
    ) {
        self.reset_method_keywords(method_type, public_variables_declared);
        let mut variables_in_method = Vec::new();
        match method_type {
            MethodType::Routine => {
                self.build_routine_method_arguments(
                    node,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    false,
                );
                self.build_subroutine_set_variables(
                    node,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    method_range,
                );
                let (unresolved_method_refs, unresolved_oref_method_refs) =
                    self.get_method_dependencies(node, content, false, class_name, method_range);
                (
                    false,
                    false,
                    variables_in_method,
                    unresolved_method_refs,
                    unresolved_oref_method_refs,
                )
            }
            MethodType::Subroutine(is_public) | MethodType::DottedSubroutine(is_public) => {
                let is_public_changed = self.is_public != is_public;
                self.build_routine_method_arguments(
                    node,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    false,
                );
                self.build_subroutine_set_variables(
                    node,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    method_range,
                );
                let (unresolved_method_refs, unresolved_oref_method_refs) =
                    self.get_method_dependencies(node, content, false, class_name, method_range);

                (
                    false,
                    is_public_changed,
                    variables_in_method,
                    unresolved_method_refs,
                    unresolved_oref_method_refs,
                )
            }
            MethodType::Procedure(is_public) => {
                let is_public_changed = self.is_public != is_public;
                self.build_procedure_set_variables(
                    node,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    class_is_procedure_block,
                    false,
                );
                self.build_routine_method_arguments(
                    node,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    true,
                );
                let (unresolved_method_refs, unresolved_oref_method_refs) =
                    self.get_method_dependencies(node, content, false, class_name, method_range);
                (
                    false,
                    is_public_changed,
                    variables_in_method,
                    unresolved_method_refs,
                    unresolved_oref_method_refs,
                )
            }
            MethodType::ClassMethod | MethodType::InstanceMethod => {
                let (is_final_changed, is_public_changed) =
                    self.build_method_keywords(node, content, class_is_final, old_class_is_final);
                self.build_class_method_arguments(
                    node,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    class_is_procedure_block,
                );
                self.build_procedure_set_variables(
                    node,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    class_is_procedure_block,
                    true,
                );
                let (unresolved_method_refs, unresolved_oref_method_refs) =
                    self.get_method_dependencies(node, content, true, class_name, method_range);
                return (
                    is_final_changed,
                    is_public_changed,
                    variables_in_method,
                    unresolved_method_refs,
                    unresolved_oref_method_refs,
                );
            }
        }
    }

    pub fn reset_method_keywords(
        &mut self,
        method_type: MethodType,
        public_variables_declared: HashSet<String>,
    ) {
        self.method_type = method_type;
        match method_type {
            MethodType::Routine => {
                self.return_type = None;
                self.variables.clear();
                self.public_variables_declared = public_variables_declared;
            }
            MethodType::Procedure(is_public)
            | MethodType::Subroutine(is_public)
            | MethodType::DottedSubroutine(is_public) => {
                self.return_type = None;
                self.variables.clear();
                self.public_variables_declared = public_variables_declared;
                self.is_public = is_public;
            }
            MethodType::ClassMethod | MethodType::InstanceMethod => {
                self.return_type = None;
                self.variables.clear();
                self.is_public = true;
                self.is_procedure_block = None;
                self.language = None;
                self.public_variables_declared = HashSet::new();
                self.code_mode = CodeMode::Code;
                self.is_final = None;
            }
        }
    }

    fn build_method_keywords(
        &mut self,
        node: Node,
        content: &str,
        class_is_final: Option<bool>,
        old_class_is_final: Option<bool>,
    ) -> (bool, bool) {
        // reset keywords to default based on method type
        let mut is_final_changed = false;
        let mut privacy_changed = false;
        {
            let query = method_keyword_query();
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(query, node, content.as_bytes());
            let keyword_idx = query.capture_index_for_name("keyword");
            let returntype_idx = query.capture_index_for_name("returntype");
            let old_is_public = self.is_public.clone();
            let old_is_final = self.is_final.clone();
            let mut return_type_parameters = Vec::new();
            let mut saw_first_return_type = false;
            let mut return_type_id = None;
            while let Some(query_match) = iter.next() {
                let mut i = 0;
                while i < query_match.captures.len() {
                    let capture = &query_match.captures[i];
                    if keyword_idx == Some(capture.index) {
                        if let Some(keyword_str) =
                            get_string_at_byte_range(content, capture.node.byte_range())
                        {
                            let (not, keyword_name, values) =
                                get_keyword_and_value(keyword_str.as_str());
                            if keyword_name == "final" && !class_is_final.unwrap_or(false) {
                                if not {
                                    self.is_final = Some(false);
                                } else {
                                    self.is_final = Some(true);
                                }
                            } else if keyword_name == "private" {
                                if not {
                                    self.is_public = true;
                                } else {
                                    self.is_public = false;
                                }
                            } else if keyword_name == "procedureblock" {
                                if let Some(value) = values.get(0).copied() {
                                    if value == "1" {
                                        self.is_procedure_block = Some(true);
                                    } else if value == "0" {
                                        self.is_procedure_block = Some(false);
                                    }
                                } else {
                                    self.is_procedure_block = Some(true);
                                }
                            } else if keyword_name == "codemode" {
                                let Some(value) = values.get(0).copied() else {
                                    eprintln!("Expected a value for language keyword, got: None");
                                    i += 1;
                                    continue;
                                };
                                if value == "call" {
                                    self.code_mode = CodeMode::Call;
                                } else if value == "code" {
                                    self.code_mode = CodeMode::Code;
                                } else if value == "expression" {
                                    self.code_mode = CodeMode::Expression;
                                } else if value == "objectgenerator" {
                                    self.code_mode = CodeMode::ObjectGenerator;
                                }
                            } else if keyword_name == "publiclist" {
                                for variable in values {
                                    self.public_variables_declared.insert(variable.to_string());
                                }
                            } else if keyword_name == "language" {
                                if let Some(value) = values.get(0).copied() {
                                    if value == "objectscript" {
                                        self.language = Some(Language::Objectscript);
                                    } else if value == "tsql" {
                                        self.language = Some(Language::TSql);
                                    } else if value == "ispl" {
                                        self.language = Some(Language::ISpl);
                                    } else if value == "python" {
                                        self.language = Some(Language::Python);
                                    }
                                }
                            }
                        }
                        i += 1;
                        continue;
                    } else if returntype_idx == Some(capture.index) {
                        let return_type_node = capture.node;
                        let Some(typename) =
                            get_string_at_byte_range(content, return_type_node.byte_range())
                        else {
                            i += 1;
                            continue;
                        };
                        if !saw_first_return_type {
                            return_type_id = Some(find_return_type(typename));
                            saw_first_return_type = true;
                        } else {
                            return_type_parameters.push(typename);
                        }
                        i += 1;
                        continue;
                    }
                    i += 1;
                }
            }
            if let Some(typename_id) = return_type_id {
                let typename = TypeName {
                    ret_type: typename_id,
                    parameters: return_type_parameters,
                };
                self.return_type = Some(typename);
            }
            let old_final_keyword_res = old_is_final.unwrap_or(old_class_is_final.unwrap_or(false));
            let new_final_keyword = self.is_final.unwrap_or(class_is_final.unwrap_or(false));
            if old_final_keyword_res != new_final_keyword {
                is_final_changed = true;
            }
            if old_is_public != self.is_public {
                privacy_changed = true;
            }
        }
        (is_final_changed, privacy_changed)
    }
}
