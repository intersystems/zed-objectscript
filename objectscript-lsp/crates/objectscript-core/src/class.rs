use crate::common::{
    get_keyword_and_value, get_member_name_from_root, get_node_children, get_string_at_byte_range,
};
use crate::method::initial_build_method;
use crate::parse_structures::{Class, Language, Method, MethodRef, MethodType};
use std::collections::HashMap;
use tree_sitter::{Node, Range};

/// Determines if a node kind terminates a routine method scope.
fn is_rtn_method_end(node_str: &str, compiled_header: bool) -> bool {
    if compiled_header {
        return node_str == "command_quit"
            || node_str == "procedure"
            || node_str == "tag_statement";
    } else {
        return node_str == "command_quit" || node_str == "procedure";
    }
}
impl Class {
    /// Creates a new `Class` with the given name and empty semantic state.
    ///
    /// Inheritance/imports/keywords/members are initialized to defaults; `active` is `true`.
    pub fn new(name: String, is_rtn: bool) -> Self {
        Self {
            name,
            imports: Vec::new(),
            inherited_classes: Vec::new(),
            inheritance_direction: "left".to_string(),
            is_procedure_block: None,
            default_language: None,
            methods: HashMap::new(),
            private_properties: HashMap::new(),
            public_properties: HashMap::new(),
            parameters: HashMap::new(),
            active: true,
            is_rtn,
            next_method_id: 0,
        }
    }

    /// Resets this `Class` to a clean state and sets its `name` and `active` flag.
    ///
    /// Clears imports/inheritance/keywords/methods/properties/params/method_calls and restores
    /// default inheritance direction to `"left"`.
    pub fn clear(&mut self, class_name: String, active: bool) {
        self.name = class_name;
        self.imports = Vec::new();
        self.inherited_classes = Vec::new();
        self.inheritance_direction = "left".to_string();
        self.is_procedure_block = None;
        self.default_language = None;
        self.methods = HashMap::new();
        self.private_properties = HashMap::new();
        self.public_properties = HashMap::new();
        self.parameters = HashMap::new();
        self.active = active;
        self.next_method_id = 0;
    }

    /// Allocates and returns the next sequential method ID for this class.
    pub fn next_id(&mut self) -> usize {
        let id = self.next_method_id;
        self.next_method_id += 1;
        id
    }

    /// Resets this `Class` to a clean state for methods that have been changed.
    ///
    /// Clears parts of the class that has been changed
    pub fn partial_clear(
        &mut self,
        class_name: String,
        active: bool,
        methods_to_remove: Vec<String>,
    ) {
        self.name = class_name;
        self.imports = Vec::new();
        self.inherited_classes = Vec::new();
        self.inheritance_direction = "left".to_string();
        self.is_procedure_block = None;
        self.default_language = None;
        for method_name in methods_to_remove {
            self.methods.remove(&method_name);
        }
        self.private_properties = HashMap::new();
        self.public_properties = HashMap::new();
        self.parameters = HashMap::new();
        self.active = active;
    }

    /// Extracts class keywords (ProcedureBlock, Language, InheritanceDirection) and collects
    /// method definitions from the class body. Does not compute imports, include files, or
    /// inherited/transitive semantics; those are handled later.
    ///
    /// Returns the parsed methods and their source ranges.
    pub fn cls_initial_build(
        &mut self,
        node: Node,
        content: &str,
        methods: &mut Vec<(Method, Range, usize)>,
    ) {
        let class_children = get_node_children(node);
        if class_children.len() < 2 {
            eprintln!(
                "initial_build: expected class_definition node, got kind={} named_children={}",
                node.kind(),
                class_children.len()
            );
            return;
        }
        // skip keyword_class and class_name
        for node in class_children.iter().skip(2) {
            match node.kind() {
                "class_keyword" => {
                    let Some(class_keyword_str) =
                        get_string_at_byte_range(content, node.byte_range())
                    else {
                        eprintln!(
                            "Couldn't get string class keyword node, continuing (initial build)"
                        );
                        continue;
                    };
                    let (not, keyword_name, values) =
                        get_keyword_and_value(class_keyword_str.as_str());
                    if keyword_name == "procedureblock" {
                        if not {
                            self.is_procedure_block = Some(false);
                        } else {
                            self.is_procedure_block = Some(true);
                        }
                    } else if keyword_name == "language" {
                        let Some(value) = values.get(0).copied() else {
                            eprintln!("Error: Expected a value for language keyword, got: None");
                            continue;
                        };
                        if value == "objectscript" {
                            self.default_language = Some(Language::Objectscript);
                        } else if value == "tsql" {
                            self.default_language = Some(Language::TSql);
                        } else {
                            eprintln!(
                                "Error: Expected class keyword language to be 'objectscript' or 'tsql', got: {}",
                                value
                            );
                            continue;
                        }
                    } else if keyword_name == "inheritance" {
                        let Some(value) = values.get(0).copied() else {
                            eprintln!("Error: Expected a value for inheritance keyword, got: None");
                            continue;
                        };
                        if value == "right" {
                            self.inheritance_direction = "right".to_string();
                        } else if value == "left" {
                            self.inheritance_direction = "left".to_string();
                        } else {
                            eprintln!(
                                "Error: Expected class keyword inheritance to be 'right' or 'left', got: {}",
                                value
                            );
                            continue;
                        }
                    }
                }
                "class_body" => {
                    let class_statements = get_node_children(node.clone());

                    // each child is a class statement
                    for class_statement in class_statements {
                        let Some(statement_type) = class_statement.named_child(0) else {
                            eprintln!(
                                "Error: class statement node {:?} has no child at index 0",
                                class_statement.kind()
                            );
                            continue;
                        };
                        match statement_type.kind() {
                            "method" | "classmethod" => {
                                let Some((method, method_range)) =
                                    self.handle_class_statement_method(statement_type, content)
                                else {
                                    eprintln!(
                                        "Error: Failed to get method from handle_class_statement_method"
                                    );
                                    continue;
                                };
                                let method_id = self.next_id();
                                methods.push((method, method_range, method_id));
                            }
                            _ => {
                                continue;
                            }
                        }
                    }
                }
                _ => {
                    continue;
                }
            }
        }
    }

    /// Parses the tree of an ObjectScript Routine file. Extracts subroutines and procedures,
    /// and builds corresponding Method structs to semantically represent them.
    ///
    /// Returns the parsed methods and their ranges.
    pub fn rtn_initial_build(
        &mut self,
        node: Node,
        content: &str,
        methods: &mut Vec<(Method, Range, usize)>,
    ) {
        let Some(routine_name) = get_member_name_from_root(content, node, true) else {
            return;
        };
        let mut curr_routine_child = node.named_child(0);
        while let Some(routine_child) = curr_routine_child {
            match routine_child.kind() {
                "routine_definition" | "compiled_header" => {
                    let mut saw_first_tag_statement = false;
                    let is_compiled_header = routine_child.kind() == "compiled_header";
                    // get statement siblings until one is tag_statement or procedure
                    let mut next_sibling = routine_child.next_named_sibling();
                    let routine_start_point = routine_child.start_position();
                    let routine_start_byte = routine_child.start_byte();
                    let mut routine_scope_end_point = node.end_position();
                    let mut routine_scope_end_byte = node.end_byte();
                    while let Some(sib) = next_sibling {
                        if sib.kind() == "statement" {
                            if let Some(future_statement_type) = sib.named_child(0) {
                                if !is_compiled_header || saw_first_tag_statement {
                                    if is_rtn_method_end(
                                        future_statement_type.kind(),
                                        is_compiled_header,
                                    ) {
                                        break;
                                    }
                                } else if future_statement_type.kind() == "tag_statement" {
                                    saw_first_tag_statement = true
                                }
                            }
                        }
                        routine_scope_end_point = sib.end_position();
                        routine_scope_end_byte = sib.end_byte();
                        next_sibling = sib.next_named_sibling();
                    }
                    let routine_range = Range {
                        start_byte: routine_start_byte,
                        start_point: routine_start_point,
                        end_point: routine_scope_end_point,
                        end_byte: routine_scope_end_byte,
                    };
                    let routine_method = Method::new(
                        routine_name.clone(),
                        Some(false),
                        None,
                        crate::parse_structures::CodeMode::Code,
                        true,
                        None,
                        Vec::new(),
                        MethodType::Routine,
                    );
                    let method_id = self.next_id();
                    methods.push((routine_method, routine_range, method_id));
                    curr_routine_child = routine_child.next_named_sibling();
                }
                "statement" => {
                    let Some(statement_type) = routine_child.named_child(0) else {
                        eprintln!("Error: Expected Statement node to have child at index 0");
                        curr_routine_child = routine_child.next_named_sibling();
                        continue;
                    };
                    if statement_type.kind() == "tag_statement" {
                        let mut is_public = true;
                        let Some(tag) = statement_type.named_child(0) else {
                            eprintln!("Error: expected tag statement node to have child at node 0");
                            curr_routine_child = routine_child.next_named_sibling();
                            continue;
                        };

                        let Some(name) = get_string_at_byte_range(content, tag.byte_range()) else {
                            curr_routine_child = routine_child.next_named_sibling();
                            continue;
                        };
                        if let Some(child) = statement_type
                            .named_child((statement_type.named_child_count() - 1) as u32)
                        {
                            match child.kind() {
                                "keyword_methodimpl" => {
                                    eprintln!(
                                        "TODO: Verify if there is anything to be done for methodimpl keyword"
                                    );
                                }
                                "keyword_private" => {
                                    is_public = false;
                                }
                                _ => {}
                            }
                        }
                        // get statement siblings until one is tag_statement or procedure
                        let mut next_sibling = routine_child.next_named_sibling();
                        let subroutine_start_point = statement_type.start_position();
                        let subroutine_start_byte = statement_type.start_byte();
                        let mut subroutine_scope_end_point = node.end_position();
                        let mut subroutine_scope_end_byte = node.end_byte();
                        while let Some(sib) = next_sibling {
                            if sib.kind() == "statement" {
                                if let Some(future_statement_type) = sib.named_child(0) {
                                    if is_rtn_method_end(future_statement_type.kind(), false) {
                                        break;
                                    }
                                }
                            }
                            subroutine_scope_end_point = sib.end_position();
                            subroutine_scope_end_byte = sib.end_byte();
                            next_sibling = sib.next_named_sibling();
                        }
                        let subroutine_range = Range {
                            start_byte: subroutine_start_byte,
                            start_point: subroutine_start_point,
                            end_point: subroutine_scope_end_point,
                            end_byte: subroutine_scope_end_byte,
                        };
                        let subroutine_method = Method::new(
                            name.clone(),
                            Some(false),
                            None,
                            crate::parse_structures::CodeMode::Code,
                            is_public,
                            None,
                            Vec::new(),
                            MethodType::Subroutine,
                        );
                        let method_id = self.next_id();
                        methods.push((subroutine_method, subroutine_range, method_id));
                        curr_routine_child = routine_child.next_named_sibling();
                        // subroutine
                    } else if statement_type.kind() == "procedure" {
                        let Some(tag) = statement_type.named_child(0) else {
                            eprintln!(
                                "Expected procedure node to have a child at index 0, aborting initial_build_procedure"
                            );
                            curr_routine_child = routine_child.next_named_sibling();
                            continue;
                        };
                        let Some(name) = get_string_at_byte_range(content, tag.byte_range()) else {
                            curr_routine_child = routine_child.next_named_sibling();
                            continue;
                        };
                        let procedure_range = statement_type.range();
                        let mut is_public = false;
                        let mut public_variables_declared = Vec::new();
                        let procedure_children = get_node_children(statement_type);
                        for procedure_statement in procedure_children {
                            match procedure_statement.kind() {
                                "procedure_pub_vars" => {
                                    let variables = get_node_children(procedure_statement);
                                    for var in variables {
                                        let Some(var_name) =
                                            get_string_at_byte_range(content, var.byte_range())
                                        else {
                                            continue;
                                        };
                                        public_variables_declared.push(var_name)
                                    }
                                }
                                "keyword_public" => {
                                    is_public = true;
                                }
                                _ => {
                                    continue;
                                }
                            }
                        }
                        let procedure_method = Method::new(
                            name.clone(),
                            Some(true),
                            None,
                            crate::parse_structures::CodeMode::Code,
                            is_public,
                            None,
                            public_variables_declared,
                            MethodType::Procedure,
                        );
                        let method_id = self.next_id();
                        methods.push((procedure_method, procedure_range, method_id));
                        curr_routine_child = routine_child.next_named_sibling();
                    } else {
                        curr_routine_child = routine_child.next_named_sibling();
                        continue;
                    }
                }
                _ => {
                    curr_routine_child = routine_child.next_named_sibling();
                    continue;
                }
            }
        }
    }

    /// Performs the first-pass parse of an ObjectScript Routine or Cls Document into this `Class`.
    ///
    pub fn initial_build(
        &mut self,
        node: Node,
        content: &str,
        is_rtn: bool,
    ) -> Vec<(Method, Range, usize)> {
        let mut methods = Vec::new();
        if !is_rtn {
            self.cls_initial_build(node, content, &mut methods);
        } else {
            self.rtn_initial_build(node, content, &mut methods);
        }
        methods
    }

    /// Parses a `method` or `classmethod` node and returns the corresponding `Method` and its `Range`.
    ///
    /// Supports instance methods (`method`) and class methods (`classmethod`). Logs and returns
    /// `None` for unsupported statement kinds or malformed syntax nodes.
    fn handle_class_statement_method(
        &mut self,
        node: Node,
        content: &str,
    ) -> Option<(Method, Range)> {
        let Some(method_definition) = node.named_child(1) else {
            eprintln!(
                "Error: Failed to get method definition from node {:?}. Aborting handle_class_statement_method.",
                node.kind()
            );
            return None;
        };
        match node.kind() {
            "method" => {
                initial_build_method(method_definition, MethodType::InstanceMethod, content)
            }
            "classmethod" => {
                initial_build_method(method_definition, MethodType::ClassMethod, content)
            }
            _ => {
                eprintln!(
                    "Error: expected method or classmethod node, but got {:?}, aborting handle_class_statement_method.",
                    node.kind()
                );
                None
            }
        }
    }

    /// Returns the `PublicMethodId` for `method_name`, if this class declares it as public.
    ///
    /// Logs and returns `None` if the method is not present in `public_methods`.
    pub fn get_method_ref(&self, method_name: &str) -> Option<&MethodRef> {
        self.methods.get(method_name)
    }
}
