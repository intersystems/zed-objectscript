use crate::common::{
    get_keyword_and_value, get_node_children, get_parameter_name, get_procedure_info,
    get_property_name, get_routine_method_range, get_string_at_byte_range, get_subroutine_info,
};

use crate::parse_structures::{
    Class, ClassId, Language, MemberType, Method, MethodId, MethodRef, MethodType, Parameter,
    ParameterId, ParameterRef, Property, PropertyId, PropertyRef,
};
use std::collections::{HashMap, HashSet};
use tree_sitter::{
    Language as TsLanguage, Node, Query, QueryCursor, Range, StreamingIterator, Tree,
};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;

impl Class {
    /// Creates a new `Class` with the given name and empty semantic state.
    ///
    /// Inheritance/imports/keywords/members are initialized to defaults; `active` is `true`.
    pub fn new(name: String, is_rtn: bool) -> Self {
        Self {
            name,
            imports: Vec::new(),
            inherited_classes: Vec::new(),
            inheritance_direction: None,
            is_procedure_block: None,
            default_language: None,
            methods: HashMap::new(),
            properties: HashMap::new(),
            parameters: HashMap::new(),
            active: true,
            is_rtn,
            next_method_id: 0,
            next_parameter_id: 0,
            next_property_id: 0,
            is_final: None,
        }
    }

    pub fn reset_keywords(&mut self) {
        self.active = true;
        self.is_final = None;
        self.inherited_classes = Vec::new();
        self.inheritance_direction = None;
        self.is_procedure_block = None;
        self.default_language = None;
    }

    /// Resets this `Class` to a clean state and sets its `name` and `active` flag.
    ///
    /// Clears imports/inheritance/keywords/methods/properties/params/method_calls and restores
    /// default inheritance direction to `"left"`.
    pub fn clear(&mut self, class_name: String, active: bool) {
        self.name = class_name;
        self.imports = Vec::new();
        self.inherited_classes = Vec::new();
        self.inheritance_direction = None;
        self.is_procedure_block = None;
        self.default_language = None;
        self.methods = HashMap::new();
        self.properties = HashMap::new();
        self.parameters = HashMap::new();
        self.active = active;
        self.next_method_id = 0;
        self.next_parameter_id = 0;
        self.next_property_id = 0;
        self.is_final = None;
    }

    /// Allocates and returns the next sequential method ID for this class.
    pub fn get_next_method_id(&mut self) -> usize {
        let id = self.next_method_id;
        self.next_method_id += 1;
        id
    }

    /// Allocates and returns the next sequential parameter ID for this class.
    pub fn get_next_parameter_id(&mut self) -> usize {
        let id = self.next_parameter_id;
        self.next_parameter_id += 1;
        id
    }

    /// Allocates and returns the next sequential property ID for this class.
    pub fn get_next_property_id(&mut self) -> usize {
        let id = self.next_property_id;
        self.next_property_id += 1;
        id
    }

    /// Given a tree, parse the children, and add any imports
    pub fn build_imports(&mut self, tree: &Tree, content: &str) {
        let source_file_children = get_node_children(tree.root_node());
        for class_child in source_file_children {
            if class_child.kind() == "import_code" {
                let import_code_children = get_node_children(class_child);
                for import_child in import_code_children {
                    if import_child.kind() == "class_name" {
                        let Some(identifier) = import_child.named_child(0) else {
                            eprintln!(
                                "Error: class name child should exist at index 0, must update parsing in get_imports_for_class"
                            );
                            continue;
                        };
                        if let Some(name) =
                            get_string_at_byte_range(content, identifier.byte_range())
                        {
                            self.imports.push(name);
                        }
                    }
                }
            }
        }
    }

    /// Clear any stale class members from this class and rebuild the class keywords.
    /// Returns HashSets of methods to remove, methods to add, and the new class keywords.
    pub fn build_class(
        &mut self,
        root_node: Node,
        content: &str,
        is_rtn: bool,
        class_id: &ClassId,
        class_range: Range,
        class_name: &String,
    ) -> (
        bool,            // Whether inherited classes changed
        bool, // class name changed                                            // class name
        HashSet<String>, // stale methods
        HashMap<String, (Method, Range, MethodRef, HashSet<String>)>, // new methods
        HashMap<String, (Property, Range, PropertyRef)>, // new properties
        HashMap<String, (Parameter, Range, ParameterRef)>, // new parameters
        Vec<String>, // new inherited classes
        HashMap<String, (Range, MethodType, HashSet<String>)>, // all methods info
        HashSet<PropertyRef>, // old properties info
        HashSet<ParameterRef>, // old parameters info
    ) {
        // (inheritance_changed, recompute_inheritance_keyword, class_name_changed, class_is_final, class_is_procedure_block, class_name)
        // // stale methods, stale prop, stale param
        // new methods, new prop, new param
        let language: &TsLanguage = if is_rtn {
            &LANGUAGE_OBJECTSCRIPT_ROUTINE.into()
        } else {
            &LANGUAGE_OBJECTSCRIPT_UDL.into()
        };
        self.reset_keywords();
        let mut inherited_count = 0;
        let mut class_name_changed = &self.name != class_name;
        let mut new_methods = HashMap::new();
        let mut new_properties = HashMap::new();
        let mut new_parameters = HashMap::new();
        let mut all_methods = HashMap::new();
        let mut old_methods: HashSet<String> = self.methods.keys().cloned().collect();
        let old_properties: HashSet<PropertyRef> = self.properties.values().copied().collect();
        let old_parameters: HashSet<ParameterRef> = self.parameters.values().copied().collect();
        let mut inheritance_changed = false;
        let old_inheritance_direction = self.inheritance_direction.clone();
        let mut inherited_classes = Vec::new();
        // NOTE: right now, properties and parameters are not incremental.. they are so small in terms of what it takes to rebuild that it doesn't make sense to incrementally build them atm
        self.properties.clear();
        self.parameters.clear();
        self.next_property_id = 0;
        self.next_parameter_id = 0;
        if !is_rtn {
            let header_query_str = r#"
            [
              (class_definition (class_extends (class_name (identifier) @inherits)))
              (class_definition (class_keyword) @classkeyword)
            ]"#;
            if let Ok(query) = Query::new(language, header_query_str) {
                let inherits_idx = query.capture_index_for_name("inherits");
                let keyword_idx = query.capture_index_for_name("classkeyword");
                let mut cursor = QueryCursor::new();
                let mut iter = cursor.matches(&query, root_node, content.as_bytes());

                while let Some(query_match) = iter.next() {
                    let mut i = 0;
                    while i < query_match.captures.len() {
                        let capture = &query_match.captures[i];
                        if inherits_idx == Some(capture.index) {
                            if let Some(inherited_cls_name) =
                                get_string_at_byte_range(content, capture.node.byte_range())
                            {
                                inherited_classes.push(inherited_cls_name.clone());
                                if let Some(old_inherited_class) =
                                    self.inherited_classes.get(inherited_count)
                                {
                                    if &inherited_cls_name != old_inherited_class {
                                        inheritance_changed = true;
                                    }
                                } else {
                                    inheritance_changed = true;
                                }
                            }
                            inherited_count += 1;
                        } else if keyword_idx == Some(capture.index)
                            && let Some(keyword_str) =
                                get_string_at_byte_range(content, capture.node.byte_range())
                        {
                            let (not, keyword_name, values) =
                                get_keyword_and_value(keyword_str.as_str());
                            if keyword_name == "procedureblock" {
                                if not {
                                    self.is_procedure_block = Some(false);
                                } else {
                                    self.is_procedure_block = Some(true);
                                }
                            } else if keyword_name == "language" {
                                if let Some(value) = values.get(0).copied() {
                                    if value == "objectscript" {
                                        self.default_language = Some(Language::Objectscript);
                                    } else if value == "tsql" {
                                        self.default_language = Some(Language::TSql);
                                    }
                                }
                            } else if keyword_name == "inheritance" {
                                if let Some(value) = values.get(0).copied() {
                                    if value == "right" {
                                        self.inheritance_direction = Some("right".to_string());
                                    } else {
                                        self.inheritance_direction = Some("left".to_string());
                                    }
                                    if self.inheritance_direction != old_inheritance_direction {
                                        inheritance_changed = true;
                                    }
                                }
                            } else if keyword_name == "final" {
                                if not {
                                    self.is_final = Some(false);
                                } else {
                                    self.is_final = Some(true);
                                }
                            }
                        }
                        i += 1;
                    }
                }
            }
        }
        let query_str = if is_rtn {
            r#"
            [(routine_definition) @routinedef  ?
            (compiled_header) @routinedef ?
            (statement (procedure)) @procedure ?
            (statement (tag_statement)) @subroutine ?]"#
        } else {
            r#"(class_definition
                        (class_body
                        (class_statement
                        [
                        (method (method_definition) @method)
                        (classmethod (method_definition) @classmethod)
                        (parameter) @parameter
                        (property) @property
                        ])
                        )
                        )"#
        };
        if let Ok(query) = Query::new(language, query_str) {
            let mut capture_indices = HashMap::new();
            if let Some(method_idx) = query.capture_index_for_name("classmethod") {
                capture_indices.insert(method_idx, MemberType::ClassMethodCall);
            }
            if let Some(routine_idx) = query.capture_index_for_name("routinedef") {
                capture_indices.insert(routine_idx, MemberType::Routine);
            }
            if let Some(subroutine_idx) = query.capture_index_for_name("subroutine") {
                capture_indices.insert(subroutine_idx, MemberType::RoutineMethodCall);
            }
            if let Some(procedure_idx) = query.capture_index_for_name("procedure") {
                capture_indices.insert(procedure_idx, MemberType::Procedure);
            }
            if let Some(method_idx) = query.capture_index_for_name("method") {
                capture_indices.insert(method_idx, MemberType::MethodDef);
            }
            if let Some(param_idx) = query.capture_index_for_name("parameter") {
                capture_indices.insert(param_idx, MemberType::Parameter);
            }
            if let Some(prop_idx) = query.capture_index_for_name("property") {
                capture_indices.insert(prop_idx, MemberType::RelativeProperty);
            }
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(&query, root_node, content.as_bytes());

            while let Some(query_match) = iter.next() {
                let mut i = 0;
                while i < query_match.captures.len() {
                    let capture = &query_match.captures[i];
                    if let Some(cap_type) = capture_indices.get(&capture.index) {
                        match cap_type {
                            MemberType::Procedure => {
                                let procedure_statement_node = capture.node;
                                if let Some((
                                    method_name,
                                    method_range,
                                    method_type,
                                    public_variables_declared,
                                )) = get_procedure_info(&procedure_statement_node, content)
                                {
                                    let existed = old_methods.remove(&method_name);
                                    if !existed {
                                        {
                                            let new_method_id = self.get_next_method_id();
                                            let method_ref = MethodRef {
                                                id: MethodId(new_method_id),
                                                class: *class_id,
                                                offset: None,
                                            };
                                            self.methods.insert(method_name.clone(), method_ref);
                                            let method = Method::new(
                                                method_name.clone(),
                                                public_variables_declared.clone(),
                                                method_type,
                                            );
                                            new_methods.insert(
                                                method_name.clone(),
                                                (
                                                    method,
                                                    method_range,
                                                    method_ref,
                                                    public_variables_declared.clone(),
                                                ),
                                            );
                                        }
                                    }
                                    all_methods.insert(
                                        method_name,
                                        (method_range, method_type, public_variables_declared),
                                    );
                                }
                                i += 1;
                                continue;
                            }
                            MemberType::RoutineMethodCall => {
                                let subroutine_statement_node = capture.node;
                                if let Some((method_name, method_range, method_type)) =
                                    get_subroutine_info(&subroutine_statement_node, content)
                                {
                                    let existed = old_methods.remove(&method_name);
                                    if !existed {
                                        {
                                            let new_method_id = self.get_next_method_id();
                                            let method_ref = MethodRef {
                                                id: MethodId(new_method_id),
                                                class: *class_id,
                                                offset: None,
                                            };
                                            self.methods.insert(method_name.clone(), method_ref);
                                            let method = Method::new(
                                                method_name.clone(),
                                                HashSet::new(),
                                                method_type,
                                            );
                                            new_methods.insert(
                                                method_name.clone(),
                                                (method, method_range, method_ref, HashSet::new()),
                                            );
                                        }
                                    }
                                    all_methods.insert(
                                        method_name,
                                        (method_range, method_type, HashSet::new()),
                                    );
                                }
                                i += 1;
                                continue;
                            }
                            MemberType::Routine => {
                                let routine_node = capture.node;
                                if class_name != &self.name {
                                    self.name = class_name.clone();
                                    class_name_changed = true;
                                }
                                if let Some(method_range) = get_routine_method_range(
                                    &routine_node,
                                    class_range.end_point,
                                    class_range.end_byte,
                                ) {
                                    let existed = old_methods.remove(class_name);
                                    if !existed {
                                        {
                                            let new_method_id = self.get_next_method_id();
                                            let method_ref = MethodRef {
                                                id: MethodId(new_method_id),
                                                class: *class_id,
                                                offset: None,
                                            };
                                            self.methods.insert(class_name.clone(), method_ref);
                                            let method = Method::new(
                                                class_name.clone(),
                                                HashSet::new(),
                                                MethodType::Routine,
                                            );
                                            new_methods.insert(
                                                class_name.clone(),
                                                (method, method_range, method_ref, HashSet::new()),
                                            );
                                        }
                                    }
                                    all_methods.insert(
                                        class_name.clone(),
                                        (method_range, MethodType::Routine, HashSet::new()),
                                    );
                                }

                                i += 1;
                                continue;
                            }
                            MemberType::ClassMethodCall => {
                                let method_definition_capture = capture.node;
                                if let Some(method_name_outer) =
                                    method_definition_capture.named_child(0)
                                    && let Some(method_name_node) = method_name_outer.named_child(0)
                                    && let Some(method_name) = get_string_at_byte_range(
                                        content,
                                        method_name_node.byte_range(),
                                    )
                                {
                                    let existed = old_methods.remove(&method_name);
                                    if !existed {
                                        {
                                            let new_method_id = self.get_next_method_id();
                                            let method_ref = MethodRef {
                                                id: MethodId(new_method_id),
                                                class: *class_id,
                                                offset: None,
                                            };
                                            self.methods.insert(method_name.clone(), method_ref);
                                            let method = Method::new(
                                                method_name.clone(),
                                                HashSet::new(),
                                                MethodType::ClassMethod,
                                            );
                                            new_methods.insert(
                                                method_name.clone(),
                                                (
                                                    method,
                                                    method_definition_capture.range(),
                                                    method_ref,
                                                    HashSet::new(),
                                                ),
                                            );
                                        }
                                    }
                                    all_methods.insert(
                                        method_name,
                                        (
                                            method_definition_capture.range(),
                                            MethodType::ClassMethod,
                                            HashSet::new(),
                                        ),
                                    );
                                }
                                i += 1;
                                continue;
                            }
                            MemberType::MethodDef => {
                                let method_definition_capture = capture.node;
                                if let Some(method_name_outer) =
                                    method_definition_capture.named_child(0)
                                    && let Some(method_name_node) = method_name_outer.named_child(0)
                                    && let Some(method_name) = get_string_at_byte_range(
                                        content,
                                        method_name_node.byte_range(),
                                    )
                                {
                                    let existed = old_methods.remove(&method_name);
                                    if !existed {
                                        {
                                            let new_method_id = self.get_next_method_id();
                                            let method_ref = MethodRef {
                                                id: MethodId(new_method_id),
                                                class: *class_id,
                                                offset: None,
                                            };
                                            self.methods.insert(method_name.clone(), method_ref);
                                            let method = Method::new(
                                                method_name.clone(),
                                                HashSet::new(),
                                                MethodType::InstanceMethod,
                                            );
                                            new_methods.insert(
                                                method_name.clone(),
                                                (
                                                    method,
                                                    method_definition_capture.range(),
                                                    method_ref,
                                                    HashSet::new(),
                                                ),
                                            );
                                        }
                                    }
                                    all_methods.insert(
                                        method_name,
                                        (
                                            method_definition_capture.range(),
                                            MethodType::InstanceMethod,
                                            HashSet::new(),
                                        ),
                                    );
                                }
                                i += 1;
                                continue;
                            }
                            MemberType::RelativeProperty => {
                                let property_node = capture.node;
                                if let Some(property_name) =
                                    get_property_name(&property_node, content)
                                {
                                    let new_property_id = self.get_next_property_id();
                                    let property_ref = PropertyRef {
                                        id: PropertyId(new_property_id),
                                        class: *class_id,
                                    };
                                    self.properties.insert(property_name.clone(), property_ref);
                                    let mut property = Property::new(property_name.clone());
                                    property.build_keywords(property_node, content, None, None);
                                    new_properties.insert(
                                        property_name.clone(),
                                        (property, property_node.range(), property_ref),
                                    );
                                }
                                i += 1;
                                continue;
                            }
                            MemberType::Parameter => {
                                let parameter_node = capture.node;
                                if let Some(parameter_name) =
                                    get_parameter_name(&parameter_node, content)
                                {
                                    let new_parameter_id = self.get_next_parameter_id();
                                    let parameter_ref = ParameterRef {
                                        id: ParameterId(new_parameter_id),
                                        class: *class_id,
                                    };
                                    self.parameters
                                        .insert(parameter_name.clone(), parameter_ref);

                                    let mut parameter = Parameter::new(parameter_name.clone());
                                    parameter.build_keywords(parameter_node, content, None, None);
                                    new_parameters.insert(
                                        parameter_name.clone(),
                                        (parameter, parameter_node.range(), parameter_ref),
                                    );
                                }
                                i += 1;
                                continue;
                            }
                            _ => {
                                i += 1;
                                continue;
                            }
                        }
                    }
                    eprintln!("error: didn't match type, but node is {:?}", capture.node);
                    i += 1;
                    continue;
                }
            }
        }
        self.inherited_classes = inherited_classes.clone();

        (
            inheritance_changed,
            class_name_changed,
            old_methods,
            new_methods,
            new_properties,
            new_parameters,
            inherited_classes,
            all_methods,
            old_properties,
            old_parameters,
        )
    }

    /// Returns the `PublicMethodId` for `method_name`, if this class declares it as public.
    ///
    /// Logs and returns `None` if the method is not present in `public_methods`.
    pub fn get_method_ref(&self, method_name: &str) -> Option<&MethodRef> {
        self.methods.get(method_name)
    }
}
