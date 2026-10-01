use crate::common::{
    build_argument, find_var_dependencies, get_node_children, get_string_at_byte_range,
    get_tracked_keywords, parse_line_ref, parse_return_type, range_within_range,
};
use crate::parse_structures::{
    CodeMode, Language, Method, MethodType, UnresolvedMethodRef, Variable, VariableDefType,
};

use crate::scope_structures::ScopeId;
use crate::scope_tree::ScopeTree;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use tree_sitter::{Language as TsLanguage, Node, Query, QueryCursor, Range, StreamingIterator};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;

const UDL_METHOD_ANALYSIS_QUERY: &str = r#"
    (command_set (set_argument [(set_target) (set_target_list)] @settarget (expression) @value ))
    [(class_method_call) @classmethodcall
(system_defined_function) @systemfunc
(relative_dot_method) @relativemethod
(routine_tag_call) @routine
(goto_argument) @routine
(print_argument) @routine
]"#;

const ROUTINE_METHOD_ANALYSIS_QUERY: &str = r#"
    (tag_parameter (method_arg) @arg)
    (command_set (set_argument [(set_target) (set_target_list)] @settarget (expression) @value ))
    [(class_method_call) @classmethodcall
    (system_defined_function) @systemfunc
    (relative_dot_method) @relativemethod
    (routine_tag_call) @routine
    (goto_argument) @routine
    (print_argument) @routine
    ]"#;

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

fn udl_method_analysis_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_UDL.into(),
        UDL_METHOD_ANALYSIS_QUERY,
        "UDL method analysis",
    )
}

fn routine_method_analysis_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_ROUTINE.into(),
        ROUTINE_METHOD_ANALYSIS_QUERY,
        "routine method analysis",
    )
}

#[derive(Clone, Copy)]
enum DependencyKind {
    ClassMethod,
    SystemFunction,
    RelativeMethod,
    Routine,
}

#[derive(Clone, Copy)]
struct SetFact<'tree> {
    target: Node<'tree>,
    value: Node<'tree>,
}

struct MethodQueryFacts<'tree> {
    routine_args: Vec<Node<'tree>>,
    sets: Vec<SetFact<'tree>>,
    dependencies: Vec<(DependencyKind, Node<'tree>)>,
}

fn collect_method_query_facts<'tree>(
    node: Node<'tree>,
    content: &str,
    is_class_method: bool,
    method_range: Range,
) -> MethodQueryFacts<'tree> {
    let query = if is_class_method {
        udl_method_analysis_query()
    } else {
        routine_method_analysis_query()
    };
    let mut cursor = QueryCursor::new();
    if !is_class_method {
        cursor.set_byte_range(method_range.start_byte..method_range.end_byte);
    }
    let mut matches = cursor.matches(query, node, content.as_bytes());
    let argument_idx = query.capture_index_for_name("arg");
    let set_target_idx = query.capture_index_for_name("settarget");
    let set_value_idx = query.capture_index_for_name("value");
    let class_method_idx = query.capture_index_for_name("classmethodcall");
    let system_function_idx = query.capture_index_for_name("systemfunc");
    let relative_method_idx = query.capture_index_for_name("relativemethod");
    let routine_idx = query.capture_index_for_name("routine");
    let mut facts = MethodQueryFacts {
        routine_args: Vec::new(),
        sets: Vec::new(),
        dependencies: Vec::new(),
    };

    while let Some(query_match) = matches.next() {
        let mut argument = None;
        let mut set_target = None;
        let mut set_value = None;
        for capture in query_match.captures {
            if argument_idx == Some(capture.index) {
                argument = Some(capture.node);
            } else if set_target_idx == Some(capture.index) {
                set_target = Some(capture.node);
            } else if set_value_idx == Some(capture.index) {
                set_value = Some(capture.node);
            } else if class_method_idx == Some(capture.index) {
                facts
                    .dependencies
                    .push((DependencyKind::ClassMethod, capture.node));
            } else if system_function_idx == Some(capture.index) {
                facts
                    .dependencies
                    .push((DependencyKind::SystemFunction, capture.node));
            } else if relative_method_idx == Some(capture.index) {
                facts
                    .dependencies
                    .push((DependencyKind::RelativeMethod, capture.node));
            } else if routine_idx == Some(capture.index) {
                facts
                    .dependencies
                    .push((DependencyKind::Routine, capture.node));
            }
        }
        if let (Some(target), Some(value)) = (set_target, set_value) {
            facts.sets.push(SetFact { target, value });
        }
        if let Some(argument) = argument {
            if !is_class_method {
                facts.routine_args.push(argument);
            }
        }
    }
    facts
}

pub fn build_method_struct(
    method_node: Node,
    method_type: MethodType,
    content: &str,
) -> Option<Method> {
    if method_node.kind() != "method_definition" && method_node.kind() != "clientmethod" {
        eprintln!(
            "Error: build_method_struct was called for node {:?}, but it can only be called for clientmethod or method_definition nodes",
            method_node.kind()
        );
        return None;
    }
    let mut method = Method::default();
    method.method_type = method_type;
    let method_children = get_node_children(method_node);
    for method_child in method_children {
        match method_child.kind() {
            "method_name" => {
                if let Some(method_name_node) = method_child.named_child(0)
                    && let Some(name) =
                        get_string_at_byte_range(content, method_name_node.byte_range())
                {
                    method.name = name;
                } else {
                    eprintln!("Error: failed to get method name.");
                    return None;
                }
            }
            "arguments" => {
                let argument_children = get_node_children(method_child);
                for argument in argument_children {
                    if let Some((argument_struct, argument_range)) =
                        build_argument(argument, content)
                    {
                        method.arguments.insert(
                            argument_struct.name.clone(),
                            (argument_struct, argument_range),
                        );
                    }
                }
            }
            "return_type" => {
                method.return_type = parse_return_type(method_child, content);
            }
            "expression_method_keywords"
            | "call_method_keywords"
            | "method_keywords"
            | "external_method_keywords" => {
                let tracked_keywords = get_tracked_keywords(method_child, content);
                method.is_final = tracked_keywords.is_final;
                method.language = tracked_keywords.language;
                method.is_procedure_block = tracked_keywords.procedure_block;
                method.is_public = tracked_keywords.is_public;
                method.public_variables_declared = tracked_keywords.public_variables_declared;
                method.code_mode = tracked_keywords.code_mode;
            }
            _ => continue,
        }
    }
    if method.name == "TODO".to_string() {
        eprintln!("Error: failed to parse method");
        return None;
    }
    Some(method)
}

impl Default for Method {
    fn default() -> Self {
        Self {
            method_type: MethodType::ClassMethod,
            code_mode: CodeMode::Code,
            is_final: None,
            variables: HashMap::new(),
            arguments: HashMap::new(),
            is_public: true,
            is_procedure_block: None,
            language: None,
            next_argument_id: 0,
            public_variables_declared: HashSet::new(),
            return_type: None,
            name: "TODO".to_string(),
        }
    }
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
                arguments: HashMap::new(),
                is_public: true,
                is_procedure_block: Some(false),
                language: None,
                public_variables_declared: public_variables,
                code_mode: CodeMode::Code,
                is_final: Some(true),
                next_argument_id: 0,
            },
            MethodType::ClientMethod => Self {
                method_type,
                return_type: None,
                name: method_name,
                variables: HashMap::new(),
                arguments: HashMap::new(),
                is_public: true,
                is_procedure_block: None,
                language: Some(Language::JavaScript),
                public_variables_declared: public_variables,
                code_mode: CodeMode::Code,
                is_final: None,
                next_argument_id: 0,
            },
            MethodType::Subroutine(is_public) | MethodType::DottedSubroutine(is_public) => Self {
                method_type,
                return_type: None,
                name: method_name,
                variables: HashMap::new(),
                arguments: HashMap::new(),
                is_public: is_public,
                is_procedure_block: Some(false),
                language: None,
                public_variables_declared: public_variables,
                code_mode: CodeMode::Code,
                is_final: Some(true),
                next_argument_id: 0,
            },
            MethodType::Procedure(is_public) => Self {
                method_type,
                return_type: None,
                name: method_name,
                variables: HashMap::new(),
                arguments: HashMap::new(),
                is_public: is_public,
                is_procedure_block: Some(true),
                language: None,
                public_variables_declared: public_variables,
                code_mode: CodeMode::Code,
                is_final: Some(true),
                next_argument_id: 0,
            },
            MethodType::ClassMethod | MethodType::InstanceMethod => Self {
                method_type,
                return_type: None,
                name: method_name,
                variables: HashMap::new(),
                arguments: HashMap::new(),
                is_public: true,
                is_procedure_block: None,
                language: None,
                public_variables_declared: public_variables,
                code_mode: CodeMode::Code,
                is_final: None,
                next_argument_id: 0,
            },
        };
    }

    fn build_subroutine_set_variables(
        &self,
        sets: &[SetFact<'_>],
        content: &str,
        scope_tree: &ScopeTree,
        variables_in_method: &mut Vec<(Variable, Range, Vec<String>, ScopeId)>,
        method_range: Range,
        class_name: &str,
    ) {
        {
            for set_fact in sets {
                let set_target_node = set_fact.target;
                if !range_within_range(&set_target_node.range(), &method_range) {
                    continue;
                }
                let mut var_defs = Vec::new();
                let mut var_deps = Vec::new();
                let mut var_type = VariableDefType::VariableDef;
                let var_value = set_fact.value;
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
                        "instance_variable" => {
                            if let Some(instance_var) =
                                get_string_at_byte_range(content, set_target_child.byte_range())
                                && let Some(property_name_outer) = set_target_child.named_child(0)
                                && let Some(property_name_node) = property_name_outer.named_child(0)
                                && let Some(property_name) = get_string_at_byte_range(
                                    content,
                                    property_name_node.byte_range(),
                                )
                            {
                                var_type = VariableDefType::PropertyDef((
                                    class_name.to_string(),
                                    property_name,
                                ));
                                var_defs.push((instance_var, var_range));
                            }
                        }
                        _ => {
                            // Other set targets do not define variables tracked here.
                        }
                    }
                }
                if matches!(var_type, VariableDefType::VariableDef) {
                    let (is_oref, curr_class) =
                        find_var_dependencies(var_value, content, &mut var_deps);
                    if is_oref && let Some(curr_class) = curr_class {
                        var_type = VariableDefType::OrefDef(curr_class);
                    }
                }
                for (variable_name, var_range) in &var_defs {
                    let var = Variable::new(variable_name.clone(), None, true, var_type.clone());
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
        sets: &[SetFact<'_>],
        content: &str,
        scope_tree: &ScopeTree,
        variables_in_method: &mut Vec<(Variable, Range, Vec<String>, ScopeId)>,
        class_is_procedure_block: bool,
        is_class_method: bool,
        method_range: Range,
        class_name: &str,
    ) {
        {
            for set_fact in sets {
                let mut var_defs = Vec::new();
                let mut var_deps = Vec::new();
                let mut var_type = VariableDefType::VariableDef;
                let set_target_node = set_fact.target;
                if !is_class_method && !range_within_range(&set_target_node.range(), &method_range)
                {
                    continue;
                }
                let var_value = set_fact.value;
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
                        "instance_variable" => {
                            if let Some(instance_var) =
                                get_string_at_byte_range(content, set_target_child.byte_range())
                                && let Some(property_name_outer) = set_target_child.named_child(0)
                                && let Some(property_name_node) = property_name_outer.named_child(0)
                                && let Some(property_name) = get_string_at_byte_range(
                                    content,
                                    property_name_node.byte_range(),
                                )
                            {
                                var_type = VariableDefType::PropertyDef((
                                    class_name.to_string(),
                                    property_name,
                                ));
                                var_defs.push((instance_var, var_range));
                            }
                        }
                        _ => {
                            continue;
                        }
                    }
                }
                if matches!(var_type, VariableDefType::VariableDef) {
                    let (is_oref, curr_class) =
                        find_var_dependencies(var_value, content, &mut var_deps);
                    if is_oref && let Some(curr_class) = curr_class {
                        var_type = VariableDefType::OrefDef(curr_class);
                    }
                }
                for (variable_name, var_range) in &var_defs {
                    let variable_is_public = if !is_class_method {
                        if self.public_variables_declared.contains(variable_name) {
                            true
                        } else {
                            false
                        }
                    } else {
                        self.is_procedure_block.unwrap_or(class_is_procedure_block) == false
                            || self.public_variables_declared.contains(variable_name)
                    };
                    let var = Variable::new(
                        variable_name.clone(),
                        None,
                        variable_is_public,
                        var_type.clone(),
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
        arguments: &[Node<'_>],
        content: &str,
        scope_tree: &ScopeTree,
        variables_in_method: &mut Vec<(Variable, Range, Vec<String>, ScopeId)>,
        is_procedure: bool, // false if subroutine
        method_range: Range,
    ) {
        {
            for method_arg in arguments.iter().copied() {
                if !range_within_range(&method_arg.range(), &method_range) {
                    continue;
                }
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
                        let var = Variable::new(
                            var_name,
                            None,
                            variable_is_public,
                            VariableDefType::VariableDef,
                        );
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

    fn get_method_dependencies(
        &mut self,
        facts: &MethodQueryFacts<'_>,
        content: &str,
        class_name: &str,
        method_range: Range,
    ) -> (
        HashSet<UnresolvedMethodRef>,
        HashSet<(String, String, Range, String)>,
    ) {
        let mut unresolved_method_refs = HashSet::new();
        let mut unresolved_oref_method_refs = HashSet::new();
        {
            for (kind, matched_node) in &facts.dependencies {
                let matched_node = *matched_node;
                if !range_within_range(&matched_node.range(), &method_range) {
                    continue;
                }
                if matches!(kind, DependencyKind::ClassMethod) {
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
                } else if matches!(kind, DependencyKind::SystemFunction) {
                    let Some(node_str) =
                        get_string_at_byte_range(content, matched_node.byte_range())
                    else {
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
                                && let Some(oref_method_arg_type) = oref_method_arg.named_child(0)
                                && let Some(oref_name_node) = oref_method_arg_type.named_child(0)
                                && let Some(oref_var_name) =
                                    get_string_at_byte_range(content, oref_name_node.byte_range())
                                && let Some(method_name_method_arg) = matched_node.named_child(1)
                                && let Some(method_name_arg_type) =
                                    method_name_method_arg.named_child(0)
                                && let Some(method_name_node) = method_name_arg_type.named_child(0)
                                && let Some(method_name) =
                                    get_string_at_byte_range(content, method_name_node.byte_range())
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
                                if let Some(method_name_method_arg) = matched_node.named_child(0)
                                    && let Some(method_name_arg_type) =
                                        method_name_method_arg.named_child(0)
                                    && let Some(method_name_node) =
                                        method_name_arg_type.named_child(0)
                                    && let Some(method_name) = get_string_at_byte_range(
                                        content,
                                        method_name_node.byte_range(),
                                    )
                                {
                                    if method_name_node.kind() == "string_literal" {
                                        unresolved_method_refs.insert(UnresolvedMethodRef {
                                            class: class_name.to_string(),
                                            method: method_name,
                                            offset: None,
                                            method_call_range: matched_node.range(),
                                        });
                                    }
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
                                    if method_name_node.kind() == "string_literal" {
                                        unresolved_method_refs.insert(UnresolvedMethodRef {
                                            class: classname_var,
                                            method: method_name,
                                            offset: None,
                                            method_call_range: matched_node.range(),
                                        });
                                    }
                                }
                            }
                        } else if func_name.eq_ignore_ascii_case("$system") {
                            if let Some(class_name_node) = matched_node.named_child(0)
                                && let Some(method_name_node) = matched_node.named_child(1)
                                && let Some(classname) =
                                    get_string_at_byte_range(content, class_name_node.byte_range())
                                && let Some(method_name) =
                                    get_string_at_byte_range(content, method_name_node.byte_range())
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
                } else if matches!(kind, DependencyKind::RelativeMethod) {
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
                } else if matches!(kind, DependencyKind::Routine) {
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
                                continue;
                            }
                        }
                    }
                }
            }
        }
        (unresolved_method_refs, unresolved_oref_method_refs)
    }

    /// Build Method Body
    pub fn rebuild_method(
        &mut self,
        node: Node,
        content: &str,
        scope_tree: &ScopeTree,
        method_type: MethodType,
        method_range: Range,
        _public_variables_declared: HashSet<String>, // retained for update-call compatibility
        _class_is_final: bool,
        _old_class_is_final: bool,
        class_is_procedure_block: bool,
        class_name: &str,
    ) -> (
        bool,
        bool,
        Vec<(Variable, Range, Vec<String>, ScopeId)>,
        HashSet<UnresolvedMethodRef>,
        HashSet<(String, String, Range, String)>,
    ) {
        // self.reset_method_keywords(method_type, public_variables_declared);
        let mut variables_in_method = Vec::new();
        if method_type == MethodType::ClientMethod {
            return (false, false, Vec::new(), HashSet::new(), HashSet::new());
        }
        let is_class_method = matches!(
            method_type,
            MethodType::ClassMethod | MethodType::InstanceMethod
        );
        let facts = collect_method_query_facts(node, content, is_class_method, method_range);
        match method_type {
            MethodType::ClientMethod => unreachable!("client methods return before body analysis"),
            MethodType::Routine => {
                // self.build_routine_method_arguments(
                //     &facts.routine_args,
                //     content,
                //     scope_tree,
                //     &mut variables_in_method,
                //     false,
                //     method_range,
                // );
                self.build_subroutine_set_variables(
                    &facts.sets,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    method_range,
                    class_name,
                );
                let (unresolved_method_refs, unresolved_oref_method_refs) =
                    self.get_method_dependencies(&facts, content, class_name, method_range);
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
                    &facts.routine_args,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    false,
                    method_range,
                );
                self.build_subroutine_set_variables(
                    &facts.sets,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    method_range,
                    class_name,
                );
                let (unresolved_method_refs, unresolved_oref_method_refs) =
                    self.get_method_dependencies(&facts, content, class_name, method_range);

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
                    &facts.sets,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    class_is_procedure_block,
                    false,
                    method_range,
                    class_name,
                );
                self.build_routine_method_arguments(
                    &facts.routine_args,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    true,
                    method_range,
                );
                let (unresolved_method_refs, unresolved_oref_method_refs) =
                    self.get_method_dependencies(&facts, content, class_name, method_range);
                (
                    false,
                    is_public_changed,
                    variables_in_method,
                    unresolved_method_refs,
                    unresolved_oref_method_refs,
                )
            }
            MethodType::ClassMethod | MethodType::InstanceMethod => {
                self.build_procedure_set_variables(
                    &facts.sets,
                    content,
                    scope_tree,
                    &mut variables_in_method,
                    class_is_procedure_block,
                    true,
                    method_range,
                    class_name,
                );
                let (unresolved_method_refs, unresolved_oref_method_refs) =
                    self.get_method_dependencies(&facts, content, class_name, method_range);
                return (
                    false,
                    false,
                    variables_in_method,
                    unresolved_method_refs,
                    unresolved_oref_method_refs,
                );
            }
        }
    }

    /// Allocates and returns the next sequential argument ID for this class.
    pub fn get_next_argument_id(&mut self) -> usize {
        let id = self.next_argument_id;
        self.next_argument_id += 1;
        id
    }
}
