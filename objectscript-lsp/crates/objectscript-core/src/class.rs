use crate::common::{
    get_dotted_subroutine_info, get_keyword_and_value, get_node_children, get_procedure_info,
    get_routine_method_range, get_string_at_byte_range, get_subroutine_info, ts_range_to_lsp_range,
};

use crate::foreignkey::build_foreignkey_struct;
use crate::index::build_index_struct;
use crate::method::build_method_struct;
use crate::parameter::build_parameter_struct;
use crate::parse_structures::{
    Class, ClassId, ForeignKey, ForeignKeyId, ForeignKeyRef, Index, IndexId, IndexRef,
    InheritanceDirection, Language, MemberType, Method, MethodId, MethodRef, MethodType, Parameter,
    ParameterId, ParameterRef, Projection, ProjectionId, ProjectionRef, Property, PropertyId,
    PropertyRef, Query, QueryId, QueryRef, Relationship, RelationshipId, RelationshipRef, Storage,
    StorageId, StorageRef, Trigger, TriggerId, TriggerRef, XData, XdataId, XdataRef,
};
use crate::projection::build_projection_struct;
use crate::property::build_property_struct;
use crate::query::build_query_struct;
use crate::relationship::build_relationship_struct;
use crate::storage::build_storage_struct;
use crate::trigger::build_trigger_struct;
use crate::xdata::build_xdata_struct;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range as LspRange};
use tree_sitter::{
    Language as TsLanguage, Node, Query as TsQuery, QueryCursor, Range, StreamingIterator, Tree,
};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;

#[derive(Clone, Copy)]
struct MemberCapture<'tree> {
    member_type: MemberType,
    node: Node<'tree>,
}

enum BuiltClassMember {
    Method {
        method: Method,
        range: Range,
        name_range: Range,
    },
    Property(Property, Range),
    Parameter(Parameter, Range),
    Relationship(Relationship, Range),
    ForeignKey(ForeignKey, Range),
    Query(Query, Range),
    Index(Index, Range),
    Trigger(Trigger, Range),
    XData(XData, Range),
    Projection(Projection, Range),
    Storage(Storage, Range),
}

fn build_class_member(capture: MemberCapture<'_>, content: &str) -> Option<BuiltClassMember> {
    let node = capture.node;
    match capture.member_type {
        MemberType::ClassMethodCall | MemberType::MethodDef | MemberType::ClientMethod => {
            let method_type = match capture.member_type {
                MemberType::ClassMethodCall => MethodType::ClassMethod,
                MemberType::MethodDef => MethodType::InstanceMethod,
                MemberType::ClientMethod => MethodType::ClientMethod,
                _ => unreachable!(),
            };
            let method = build_method_struct(node, method_type, content)?;
            let name_range = node.named_child(0)?.named_child(0)?.range();
            Some(BuiltClassMember::Method {
                method,
                range: node.range(),
                name_range,
            })
        }
        MemberType::RelativeProperty => build_property_struct(node, content)
            .map(|value| BuiltClassMember::Property(value, node.range())),
        MemberType::RelativeParameter => build_parameter_struct(node, content)
            .map(|value| BuiltClassMember::Parameter(value, node.range())),
        MemberType::Relationship => build_relationship_struct(node, content)
            .map(|value| BuiltClassMember::Relationship(value, node.range())),
        MemberType::Foreignkey => build_foreignkey_struct(node, content)
            .map(|value| BuiltClassMember::ForeignKey(value, node.range())),
        MemberType::Query => build_query_struct(node, content)
            .map(|value| BuiltClassMember::Query(value, node.range())),
        MemberType::Index => build_index_struct(node, content)
            .map(|value| BuiltClassMember::Index(value, node.range())),
        MemberType::Trigger => build_trigger_struct(node, content)
            .map(|value| BuiltClassMember::Trigger(value, node.range())),
        MemberType::Xdata => build_xdata_struct(node, content)
            .map(|value| BuiltClassMember::XData(value, node.range())),
        MemberType::Projection => build_projection_struct(node, content)
            .map(|value| BuiltClassMember::Projection(value, node.range())),
        MemberType::Storage => build_storage_struct(node, content)
            .map(|value| BuiltClassMember::Storage(value, node.range())),
        _ => None,
    }
}

const UDL_CLASS_MEMBER_QUERY: &str = r#"
    (class_definition
      (class_extends
        (class_name
          (identifier) @inherits)))

    (class_definition
      (class_keyword) @classkeyword)

    (class_definition
      (class_body
        (class_statement
          [
            (method (method_definition) @method)
            (classmethod (method_definition) @classmethod)
            (clientmethod) @clientmethod
            (parameter) @parameter
            (property) @property
            (relationship) @relationship
            (foreignkey) @foreignkey
            (query) @query
            (index) @index
            (trigger) @trigger
            (xdata) @xdata
            (projection) @projection
            (storage) @storage
          ])))
"#;

const ROUTINE_MEMBER_QUERY: &str = r#"
[(routine_definition) @routinedef  ?
(compiled_header) @routinedef ?
(statement (procedure)) @procedure ?
(dotted_statement (tag)) @dottedstatement ?
(statement (tag_statement)) @subroutine ?]"#;

fn cached_query(
    query: &'static OnceLock<TsQuery>,
    language: TsLanguage,
    source: &str,
    name: &str,
) -> &'static TsQuery {
    query.get_or_init(|| {
        TsQuery::new(&language, source)
            .unwrap_or_else(|error| panic!("failed to compile {name} Tree-sitter query: {error}"))
    })
}

fn udl_class_query() -> &'static TsQuery {
    static QUERY: OnceLock<TsQuery> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_UDL.into(),
        UDL_CLASS_MEMBER_QUERY,
        "UDL class member",
    )
}

fn routine_member_query() -> &'static TsQuery {
    static QUERY: OnceLock<TsQuery> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_ROUTINE.into(),
        ROUTINE_MEMBER_QUERY,
        "routine member",
    )
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
            inheritance_direction: InheritanceDirection::Left,
            is_procedure_block: true,
            default_language: Language::Objectscript,
            methods: HashMap::new(),
            properties: HashMap::new(),
            parameters: HashMap::new(),
            relationships: HashMap::new(),
            foreignkeys: HashMap::new(),
            queries: HashMap::new(),
            triggers: HashMap::new(),
            indices: HashMap::new(),
            projections: HashMap::new(),
            xdata: HashMap::new(),
            storage: HashMap::new(),
            active: true,
            is_rtn,
            next_method_id: 0,
            next_parameter_id: 0,
            next_property_id: 0,
            next_foreign_key_id: 0,
            next_index_id: 0,
            next_projection_id: 0,
            next_query_id: 0,
            next_relationship_id: 0,
            next_storage_id: 0,
            next_trigger_id: 0,
            next_xdata_id: 0,
            is_final: false,
        }
    }

    pub fn reset_keywords(&mut self) {
        self.active = true;
        self.is_final = false;
        self.inherited_classes = Vec::new();
        self.inheritance_direction = InheritanceDirection::Left;
        self.is_procedure_block = true;
        self.default_language = Language::Objectscript;
    }

    /// Resets this `Class` to a clean state and sets its `name` and `active` flag.
    ///
    /// Clears imports/inheritance/keywords/methods/properties/params/method_calls and restores
    /// default inheritance direction to `"left"`.
    pub fn clear(&mut self, class_name: String, active: bool) {
        self.name = class_name;
        self.imports = Vec::new();
        self.inherited_classes = Vec::new();
        self.inheritance_direction = InheritanceDirection::Left;
        self.is_procedure_block = true;
        self.default_language = Language::Objectscript;
        self.methods = HashMap::new();
        self.properties = HashMap::new();
        self.parameters = HashMap::new();
        self.relationships = HashMap::new();
        self.foreignkeys = HashMap::new();
        self.queries = HashMap::new();
        self.indices = HashMap::new();
        self.triggers = HashMap::new();
        self.projections = HashMap::new();
        self.xdata = HashMap::new();
        self.storage = HashMap::new();
        self.active = active;
        self.next_method_id = 0;
        self.next_parameter_id = 0;
        self.next_property_id = 0;
        self.next_relationship_id = 0;
        self.next_index_id = 0;
        self.next_foreign_key_id = 0;
        self.next_query_id = 0;
        self.next_trigger_id = 0;
        self.next_xdata_id = 0;
        self.next_projection_id = 0;
        self.next_storage_id = 0;
        self.is_final = false;
    }

    pub fn clear_metadata_for_members_rebuilt(&mut self) {
        self.next_parameter_id = 0;
        self.next_property_id = 0;
        self.next_relationship_id = 0;
        self.next_index_id = 0;
        self.next_foreign_key_id = 0;
        self.next_query_id = 0;
        self.next_trigger_id = 0;
        self.next_xdata_id = 0;
        self.next_projection_id = 0;
        self.next_storage_id = 0;
        self.properties.clear();
        self.parameters.clear();
        self.relationships.clear();
        self.foreignkeys.clear();
        self.queries.clear();
        self.indices.clear();
        self.triggers.clear();
        self.projections.clear();
        self.xdata.clear();
        self.storage.clear();
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

    /// Allocates and returns the next sequential relationship ID for this class.
    pub fn get_next_relationship_id(&mut self) -> usize {
        let id = self.next_relationship_id;
        self.next_relationship_id += 1;
        id
    }

    /// Allocates and returns the next sequential foreignkey ID for this class.
    pub fn get_next_foreignkey_id(&mut self) -> usize {
        let id = self.next_foreign_key_id;
        self.next_foreign_key_id += 1;
        id
    }

    /// Allocates and returns the next sequential query ID for this class.
    pub fn get_next_query_id(&mut self) -> usize {
        let id = self.next_query_id;
        self.next_query_id += 1;
        id
    }

    /// Allocates and returns the next sequential index ID for this class.
    pub fn get_next_index_id(&mut self) -> usize {
        let id = self.next_index_id;
        self.next_index_id += 1;
        id
    }

    /// Allocates and returns the next sequential trigger ID for this class.
    pub fn get_next_trigger_id(&mut self) -> usize {
        let id = self.next_trigger_id;
        self.next_trigger_id += 1;
        id
    }

    /// Allocates and returns the next sequential xdata ID for this class.
    pub fn get_next_xdata_id(&mut self) -> usize {
        let id = self.next_xdata_id;
        self.next_xdata_id += 1;
        id
    }

    /// Allocates and returns the next sequential projection ID for this class.
    pub fn get_next_projection_id(&mut self) -> usize {
        let id = self.next_projection_id;
        self.next_projection_id += 1;
        id
    }

    /// Allocates and returns the next sequential storage ID for this class.
    pub fn get_next_storage_id(&mut self) -> usize {
        let id = self.next_storage_id;
        self.next_storage_id += 1;
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
        HashSet<String>, // stale methods
        HashMap<String, (Method, Range, MethodRef, HashSet<String>)>, // new methods
        HashMap<String, (Property, Range, PropertyRef)>, // new properties
        HashMap<String, (Parameter, Range, ParameterRef)>, // new parameters
        HashMap<String, (Relationship, Range, RelationshipRef)>,
        HashMap<String, (ForeignKey, Range, ForeignKeyRef)>,
        HashMap<String, (Query, Range, QueryRef)>,
        HashMap<String, (Index, Range, IndexRef)>,
        HashMap<String, (Trigger, Range, TriggerRef)>,
        HashMap<String, (XData, Range, XdataRef)>,
        HashMap<String, (Projection, Range, ProjectionRef)>,
        HashMap<String, (Storage, Range, StorageRef)>,
        Vec<(String, LspRange)>, // new inherited classes
        HashMap<String, (Range, MethodType, HashSet<String>)>, // all methods info
        Vec<Diagnostic>,
    ) {
        // (inheritance_changed, recompute_inheritance_keyword, class_name_changed, class_is_final, class_is_procedure_block, class_name)
        // // stale methods, stale prop, stale param
        // new methods, new prop, new param
        self.reset_keywords();
        let mut inherited_count = 0;
        let mut new_methods = HashMap::new();
        let mut new_properties = HashMap::new();
        let mut new_parameters = HashMap::new();
        let mut new_relationships = HashMap::new();
        let mut new_foreignkeys = HashMap::new();
        let mut new_queries = HashMap::new();
        let mut new_indices = HashMap::new();
        let mut new_triggers = HashMap::new();
        let mut new_projections = HashMap::new();
        let mut new_xdata = HashMap::new();
        let mut new_storage = HashMap::new();
        let mut all_methods = HashMap::new();
        let mut diagnostics = Vec::new();
        let mut old_methods: HashSet<String> = self.methods.keys().cloned().collect();
        let mut inheritance_changed = false;
        let old_inheritance_direction = self.inheritance_direction.clone();
        let mut inherited_classes = Vec::new();
        // NOTE: right now, properties and parameters are not incremental.. they are so small in terms of what it takes to rebuild that it doesn't make sense to incrementally build them atm
        self.properties.clear();
        self.parameters.clear();
        self.relationships.clear();
        self.foreignkeys.clear();
        self.queries.clear();
        self.indices.clear();
        self.triggers.clear();
        self.projections.clear();
        self.xdata.clear();
        self.storage.clear();
        self.next_property_id = 0;
        self.next_parameter_id = 0;
        self.next_relationship_id = 0;
        self.next_foreign_key_id = 0;
        self.next_query_id = 0;
        self.next_index_id = 0;
        self.next_trigger_id = 0;
        self.next_xdata_id = 0;
        self.next_projection_id = 0;
        self.next_storage_id = 0;
        let query = if is_rtn {
            routine_member_query()
        } else {
            udl_class_query()
        };
        {
            let mut capture_indices = HashMap::new();
            if let Some(method_idx) = query.capture_index_for_name("classmethod") {
                capture_indices.insert(method_idx, MemberType::ClassMethodCall);
            }
            if let Some(client_method_idx) = query.capture_index_for_name("clientmethod") {
                capture_indices.insert(client_method_idx, MemberType::ClientMethod);
            }
            if let Some(inherits_idx) = query.capture_index_for_name("inherits") {
                capture_indices.insert(inherits_idx, MemberType::InheritedClass);
            }
            if let Some(keyword_idx) = query.capture_index_for_name("classkeyword") {
                capture_indices.insert(keyword_idx, MemberType::ClassKeyword);
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
                capture_indices.insert(param_idx, MemberType::RelativeParameter);
            }
            if let Some(prop_idx) = query.capture_index_for_name("property") {
                capture_indices.insert(prop_idx, MemberType::RelativeProperty);
            }
            if let Some(relationship_idx) = query.capture_index_for_name("relationship") {
                capture_indices.insert(relationship_idx, MemberType::Relationship);
            }
            if let Some(foreignkey_idx) = query.capture_index_for_name("foreignkey") {
                capture_indices.insert(foreignkey_idx, MemberType::Foreignkey);
            }
            if let Some(query_idx) = query.capture_index_for_name("query") {
                capture_indices.insert(query_idx, MemberType::Query);
            }
            if let Some(index_idx) = query.capture_index_for_name("index") {
                capture_indices.insert(index_idx, MemberType::Index);
            }
            if let Some(trigger_idx) = query.capture_index_for_name("trigger") {
                capture_indices.insert(trigger_idx, MemberType::Trigger);
            }
            if let Some(xdata_idx) = query.capture_index_for_name("xdata") {
                capture_indices.insert(xdata_idx, MemberType::Xdata);
            }
            if let Some(projection_idx) = query.capture_index_for_name("projection") {
                capture_indices.insert(projection_idx, MemberType::Projection);
            }
            if let Some(storage_idx) = query.capture_index_for_name("storage") {
                capture_indices.insert(storage_idx, MemberType::Storage);
            }

            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(query, root_node, content.as_bytes());

            if !is_rtn {
                let mut member_captures = Vec::new();
                while let Some(query_match) = iter.next() {
                    for capture in query_match.captures {
                        let Some(member_type) = capture_indices.get(&capture.index).copied() else {
                            continue;
                        };
                        match member_type {
                            MemberType::ClassKeyword => {
                                if let Some(keyword_str) =
                                    get_string_at_byte_range(content, capture.node.byte_range())
                                {
                                    let (not, keyword_name, values) =
                                        get_keyword_and_value(keyword_str.as_str());
                                    match keyword_name.as_str() {
                                        "procedureblock" if not => {
                                            self.is_procedure_block = false;
                                        }
                                        "language" => {
                                            if values.first().map(String::as_str) == Some("tsql") {
                                                self.default_language = Language::TSql;
                                            }
                                        }
                                        "inheritance" => {
                                            if values.first().map(String::as_str) == Some("right") {
                                                self.inheritance_direction =
                                                    InheritanceDirection::Right;
                                            }
                                            if self.inheritance_direction
                                                != old_inheritance_direction
                                            {
                                                inheritance_changed = true;
                                            }
                                        }
                                        "final" if !not => {
                                            self.is_final = true;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            MemberType::InheritedClass => {
                                if let Some(inherited_class_name) =
                                    get_string_at_byte_range(content, capture.node.byte_range())
                                {
                                    let lsp_range =
                                        ts_range_to_lsp_range(content, capture.node.range());
                                    inherited_classes
                                        .push((inherited_class_name.clone(), lsp_range));
                                    if self
                                        .inherited_classes
                                        .get(inherited_count)
                                        .map(|(name, _)| name)
                                        != Some(&inherited_class_name)
                                    {
                                        inheritance_changed = true;
                                    }
                                }
                                inherited_count += 1;
                            }
                            MemberType::ClassMethodCall
                            | MemberType::ClientMethod
                            | MemberType::MethodDef
                            | MemberType::RelativeParameter
                            | MemberType::RelativeProperty
                            | MemberType::Relationship
                            | MemberType::Foreignkey
                            | MemberType::Query
                            | MemberType::Index
                            | MemberType::Trigger
                            | MemberType::Xdata
                            | MemberType::Projection
                            | MemberType::Storage => {
                                member_captures.push(MemberCapture {
                                    member_type,
                                    node: capture.node,
                                });
                            }
                            _ => {}
                        }
                    }
                }

                let built_members: Vec<BuiltClassMember> = member_captures
                    .into_par_iter()
                    .filter_map(|capture| build_class_member(capture, content))
                    .collect();

                for built_member in built_members {
                    match built_member {
                        BuiltClassMember::Method {
                            method,
                            range,
                            name_range,
                        } => {
                            let method_name = method.name.clone();
                            let method_type = method.method_type;
                            let public_variables = method.public_variables_declared.clone();
                            let existed = old_methods.remove(&method_name);
                            if all_methods.contains_key(&method_name) {
                                diagnostics.push(Diagnostic {
                                    range: ts_range_to_lsp_range(content, name_range),
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    code: None,
                                    code_description: None,
                                    source: Some("ObjectScript".to_string()),
                                    message: format!(
                                        "A Method named {:?} already exists in this class.",
                                        &method_name
                                    ),
                                    related_information: None,
                                    tags: None,
                                    data: None,
                                });
                            }
                            if !existed {
                                let method_ref = MethodRef {
                                    id: MethodId(self.get_next_method_id()),
                                    class: *class_id,
                                    offset: None,
                                };
                                self.methods.insert(method_name.clone(), method_ref);
                                new_methods.insert(
                                    method_name.clone(),
                                    (method, range, method_ref, public_variables.clone()),
                                );
                            }
                            all_methods.insert(method_name, (range, method_type, public_variables));
                        }
                        BuiltClassMember::Property(property, range) => {
                            let name = property.name.clone();
                            let member_ref = PropertyRef {
                                id: PropertyId(self.get_next_property_id()),
                                class: *class_id,
                            };
                            self.properties.insert(name.clone(), member_ref);
                            new_properties.insert(name, (property, range, member_ref));
                        }
                        BuiltClassMember::Parameter(parameter, range) => {
                            let name = parameter.name.clone();
                            let member_ref = ParameterRef {
                                id: ParameterId(self.get_next_parameter_id()),
                                class: *class_id,
                            };
                            self.parameters.insert(name.clone(), member_ref);
                            new_parameters.insert(name, (parameter, range, member_ref));
                        }
                        BuiltClassMember::Relationship(relationship, range) => {
                            let name = relationship.name.clone();
                            let member_ref = RelationshipRef {
                                id: RelationshipId(self.get_next_relationship_id()),
                                class: *class_id,
                            };
                            self.relationships.insert(name.clone(), member_ref);
                            new_relationships.insert(name, (relationship, range, member_ref));
                        }
                        BuiltClassMember::ForeignKey(foreignkey, range) => {
                            let name = foreignkey.name.clone();
                            let member_ref = ForeignKeyRef {
                                id: ForeignKeyId(self.get_next_foreignkey_id()),
                                class: *class_id,
                            };
                            self.foreignkeys.insert(name.clone(), member_ref);
                            new_foreignkeys.insert(name, (foreignkey, range, member_ref));
                        }
                        BuiltClassMember::Query(query, range) => {
                            let name = query.name.clone();
                            let member_ref = QueryRef {
                                id: QueryId(self.get_next_query_id()),
                                class: *class_id,
                            };
                            self.queries.insert(name.clone(), member_ref);
                            new_queries.insert(name, (query, range, member_ref));
                        }
                        BuiltClassMember::Index(index, range) => {
                            let name = index.name.clone();
                            let member_ref = IndexRef {
                                id: IndexId(self.get_next_index_id()),
                                class: *class_id,
                            };
                            self.indices.insert(name.clone(), member_ref);
                            new_indices.insert(name, (index, range, member_ref));
                        }
                        BuiltClassMember::Trigger(trigger, range) => {
                            let name = trigger.name.clone();
                            let member_ref = TriggerRef {
                                id: TriggerId(self.get_next_trigger_id()),
                                class: *class_id,
                            };
                            self.triggers.insert(name.clone(), member_ref);
                            new_triggers.insert(name, (trigger, range, member_ref));
                        }
                        BuiltClassMember::XData(xdata, range) => {
                            let name = xdata.name.clone();
                            let member_ref = XdataRef {
                                id: XdataId(self.get_next_xdata_id()),
                                class: *class_id,
                            };
                            self.xdata.insert(name.clone(), member_ref);
                            new_xdata.insert(name, (xdata, range, member_ref));
                        }
                        BuiltClassMember::Projection(projection, range) => {
                            let name = projection.name.clone();
                            let member_ref = ProjectionRef {
                                id: ProjectionId(self.get_next_projection_id()),
                                class: *class_id,
                            };
                            self.projections.insert(name.clone(), member_ref);
                            new_projections.insert(name, (projection, range, member_ref));
                        }
                        BuiltClassMember::Storage(storage, range) => {
                            let name = storage.name.clone();
                            let member_ref = StorageRef {
                                id: StorageId(self.get_next_storage_id()),
                                class: *class_id,
                            };
                            self.storage.insert(name.clone(), member_ref);
                            new_storage.insert(name, (storage, range, member_ref));
                        }
                    }
                }
            } else {
                while let Some(query_match) = iter.next() {
                    let mut i = 0;
                    while i < query_match.captures.len() {
                        let capture = &query_match.captures[i];
                        if let Some(cap_type) = capture_indices.get(&capture.index) {
                            match cap_type {
                                MemberType::Relationship => {
                                    let relationship_node = capture.node;
                                    if let Some(relationship) =
                                        build_relationship_struct(relationship_node, content)
                                    {
                                        let new_relationship_id = self.get_next_relationship_id();
                                        let relationship_ref = RelationshipRef {
                                            id: RelationshipId(new_relationship_id),
                                            class: *class_id,
                                        };
                                        let relationship_name = relationship.name.clone();
                                        self.relationships
                                            .insert(relationship_name.clone(), relationship_ref);
                                        new_relationships.insert(
                                            relationship_name.clone(),
                                            (
                                                relationship,
                                                relationship_node.range(),
                                                relationship_ref,
                                            ),
                                        );
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::ClassKeyword => {
                                    if let Some(keyword_str) =
                                        get_string_at_byte_range(content, capture.node.byte_range())
                                    {
                                        let (not, keyword_name, values) =
                                            get_keyword_and_value(keyword_str.as_str());
                                        if keyword_name == "procedureblock" {
                                            if not {
                                                self.is_procedure_block = false;
                                            }
                                        } else if keyword_name == "language" {
                                            if let Some(value) = values.first().map(String::as_str)
                                            {
                                                if value == "tsql" {
                                                    self.default_language = Language::TSql;
                                                }
                                            }
                                        } else if keyword_name == "inheritance" {
                                            if let Some(value) = values.first().map(String::as_str)
                                            {
                                                if value == "right" {
                                                    self.inheritance_direction =
                                                        InheritanceDirection::Right;
                                                }
                                                if self.inheritance_direction
                                                    != old_inheritance_direction
                                                {
                                                    inheritance_changed = true;
                                                }
                                            }
                                        } else if keyword_name == "final" {
                                            if !not {
                                                self.is_final = true;
                                            }
                                        }
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::InheritedClass => {
                                    if let Some(inherited_cls_name) =
                                        get_string_at_byte_range(content, capture.node.byte_range())
                                    {
                                        let lsp_range =
                                            ts_range_to_lsp_range(content, capture.node.range());
                                        inherited_classes
                                            .push((inherited_cls_name.clone(), lsp_range));
                                        // inherited_class_ranges.insert(inherited_cls_name.clone(), lsp_range);
                                        if let Some((old_inherited_class, _)) =
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
                                    i += 1;
                                    continue;
                                }
                                MemberType::Procedure => {
                                    let procedure_statement_node = capture.node;
                                    if let Some((
                                        method_name,
                                        method_name_range,
                                        method_range,
                                        method_type,
                                        public_variables_declared,
                                    )) = get_procedure_info(&procedure_statement_node, content)
                                    {
                                        let existed = old_methods.remove(&method_name);
                                        if all_methods.contains_key(&method_name) {
                                            let lsp_range =
                                                ts_range_to_lsp_range(content, method_name_range);
                                            let diagnostic = Diagnostic {
                                                range: lsp_range,
                                                severity: Some(DiagnosticSeverity::ERROR),
                                                code: None,
                                                code_description: None,
                                                source: Some("ObjectScript".to_string()),
                                                message: format!(
                                                    "A Method named {:?} already exists in this class.",
                                                    &method_name
                                                ),
                                                related_information: None,
                                                tags: None,
                                                data: None,
                                            };
                                            diagnostics.push(diagnostic);
                                        }
                                        if !existed {
                                            {
                                                let new_method_id = self.get_next_method_id();
                                                let method_ref = MethodRef {
                                                    id: MethodId(new_method_id),
                                                    class: *class_id,
                                                    offset: None,
                                                };
                                                self.methods
                                                    .insert(method_name.clone(), method_ref);
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
                                MemberType::DottedStatementTag => {
                                    let subroutine_statement_node = capture.node;
                                    if let Some((
                                        method_name,
                                        method_name_range,
                                        method_range,
                                        method_type,
                                    )) = get_dotted_subroutine_info(
                                        &subroutine_statement_node,
                                        content,
                                    ) {
                                        let existed = old_methods.remove(&method_name);
                                        if all_methods.contains_key(&method_name) {
                                            let lsp_range =
                                                ts_range_to_lsp_range(content, method_name_range);
                                            let diagnostic = Diagnostic {
                                                range: lsp_range,
                                                severity: Some(DiagnosticSeverity::ERROR),
                                                code: None,
                                                code_description: None,
                                                source: Some("ObjectScript".to_string()),
                                                message: format!(
                                                    "A Method named {:?} already exists in this class.",
                                                    &method_name
                                                ),
                                                related_information: None,
                                                tags: None,
                                                data: None,
                                            };
                                            diagnostics.push(diagnostic);
                                        }
                                        if !existed {
                                            {
                                                let new_method_id = self.get_next_method_id();
                                                let method_ref = MethodRef {
                                                    id: MethodId(new_method_id),
                                                    class: *class_id,
                                                    offset: None,
                                                };
                                                self.methods
                                                    .insert(method_name.clone(), method_ref);
                                                let method = Method::new(
                                                    method_name.clone(),
                                                    HashSet::new(),
                                                    method_type,
                                                );
                                                new_methods.insert(
                                                    method_name.clone(),
                                                    (
                                                        method,
                                                        method_range,
                                                        method_ref,
                                                        HashSet::new(),
                                                    ),
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
                                MemberType::RoutineMethodCall => {
                                    let subroutine_statement_node = capture.node;
                                    if let Some((
                                        method_name,
                                        method_name_range,
                                        method_range,
                                        method_type,
                                    )) = get_subroutine_info(&subroutine_statement_node, content)
                                    {
                                        let existed = old_methods.remove(&method_name);
                                        if all_methods.contains_key(&method_name) {
                                            let lsp_range =
                                                ts_range_to_lsp_range(content, method_name_range);
                                            let diagnostic = Diagnostic {
                                                range: lsp_range,
                                                severity: Some(DiagnosticSeverity::ERROR),
                                                code: None,
                                                code_description: None,
                                                source: Some("ObjectScript".to_string()),
                                                message: format!(
                                                    "A Method named {:?} already exists in this class.",
                                                    &method_name
                                                ),
                                                related_information: None,
                                                tags: None,
                                                data: None,
                                            };
                                            diagnostics.push(diagnostic);
                                        }
                                        if !existed {
                                            {
                                                let new_method_id = self.get_next_method_id();
                                                let method_ref = MethodRef {
                                                    id: MethodId(new_method_id),
                                                    class: *class_id,
                                                    offset: None,
                                                };
                                                self.methods
                                                    .insert(method_name.clone(), method_ref);
                                                let method = Method::new(
                                                    method_name.clone(),
                                                    HashSet::new(),
                                                    method_type,
                                                );
                                                new_methods.insert(
                                                    method_name.clone(),
                                                    (
                                                        method,
                                                        method_range,
                                                        method_ref,
                                                        HashSet::new(),
                                                    ),
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
                                                    (
                                                        method,
                                                        method_range,
                                                        method_ref,
                                                        HashSet::new(),
                                                    ),
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
                                        && let Some(method_name_node) =
                                            method_name_outer.named_child(0)
                                        && let Some(method_name) = get_string_at_byte_range(
                                            content,
                                            method_name_node.byte_range(),
                                        )
                                    {
                                        let existed = old_methods.remove(&method_name);
                                        if all_methods.contains_key(&method_name) {
                                            let lsp_range = ts_range_to_lsp_range(
                                                content,
                                                method_name_node.range(),
                                            );
                                            let diagnostic = Diagnostic {
                                                range: lsp_range,
                                                severity: Some(DiagnosticSeverity::ERROR),
                                                code: None,
                                                code_description: None,
                                                source: Some("ObjectScript".to_string()),
                                                message: format!(
                                                    "A Method named {:?} already exists in this class.",
                                                    &method_name
                                                ),
                                                related_information: None,
                                                tags: None,
                                                data: None,
                                            };
                                            diagnostics.push(diagnostic);
                                        }
                                        if !existed {
                                            {
                                                let new_method_id = self.get_next_method_id();
                                                let method_ref = MethodRef {
                                                    id: MethodId(new_method_id),
                                                    class: *class_id,
                                                    offset: None,
                                                };
                                                self.methods
                                                    .insert(method_name.clone(), method_ref);
                                                let method = build_method_struct(
                                                    method_definition_capture,
                                                    MethodType::ClassMethod,
                                                    content,
                                                )
                                                .unwrap_or_else(|| {
                                                    Method::new(
                                                        method_name.clone(),
                                                        HashSet::new(),
                                                        MethodType::ClassMethod,
                                                    )
                                                });
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
                                        && let Some(method_name_node) =
                                            method_name_outer.named_child(0)
                                        && let Some(method_name) = get_string_at_byte_range(
                                            content,
                                            method_name_node.byte_range(),
                                        )
                                    {
                                        let existed = old_methods.remove(&method_name);
                                        if all_methods.contains_key(&method_name) {
                                            let lsp_range = ts_range_to_lsp_range(
                                                content,
                                                method_name_node.range(),
                                            );
                                            let diagnostic = Diagnostic {
                                                range: lsp_range,
                                                severity: Some(DiagnosticSeverity::ERROR),
                                                code: None,
                                                code_description: None,
                                                source: Some("ObjectScript".to_string()),
                                                message: format!(
                                                    "A Method named {:?} already exists in this class.",
                                                    &method_name
                                                ),
                                                related_information: None,
                                                tags: None,
                                                data: None,
                                            };
                                            diagnostics.push(diagnostic);
                                        }
                                        if !existed {
                                            {
                                                let new_method_id = self.get_next_method_id();
                                                let method_ref = MethodRef {
                                                    id: MethodId(new_method_id),
                                                    class: *class_id,
                                                    offset: None,
                                                };
                                                self.methods
                                                    .insert(method_name.clone(), method_ref);
                                                let method = build_method_struct(
                                                    method_definition_capture,
                                                    MethodType::InstanceMethod,
                                                    content,
                                                )
                                                .unwrap_or_else(|| {
                                                    Method::new(
                                                        method_name.clone(),
                                                        HashSet::new(),
                                                        MethodType::InstanceMethod,
                                                    )
                                                });
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
                                    if let Some(property) =
                                        build_property_struct(property_node, content)
                                    {
                                        let new_property_id = self.get_next_property_id();
                                        let property_ref = PropertyRef {
                                            id: PropertyId(new_property_id),
                                            class: *class_id,
                                        };
                                        let property_name = property.name.clone();
                                        self.properties.insert(property_name.clone(), property_ref);
                                        new_properties.insert(
                                            property_name.clone(),
                                            (property, property_node.range(), property_ref),
                                        );
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::Trigger => {
                                    let trigger_node = capture.node;
                                    if let Some(trigger) =
                                        build_trigger_struct(trigger_node, content)
                                    {
                                        let new_trigger_id = self.get_next_trigger_id();
                                        let trigger_ref = TriggerRef {
                                            id: TriggerId(new_trigger_id),
                                            class: *class_id,
                                        };
                                        let trigger_name = trigger.name.clone();
                                        self.triggers.insert(trigger_name.clone(), trigger_ref);
                                        new_triggers.insert(
                                            trigger_name.clone(),
                                            (trigger, trigger_node.range(), trigger_ref),
                                        );
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::Storage => {
                                    let storage_node = capture.node;
                                    if let Some(storage) =
                                        build_storage_struct(storage_node, content)
                                    {
                                        let new_storage_id = self.get_next_storage_id();
                                        let storage_ref = StorageRef {
                                            id: StorageId(new_storage_id),
                                            class: *class_id,
                                        };
                                        let storage_name = storage.name.clone();
                                        self.storage.insert(storage_name.clone(), storage_ref);
                                        new_storage.insert(
                                            storage_name.clone(),
                                            (storage, storage_node.range(), storage_ref),
                                        );
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::Index => {
                                    let index_node = capture.node;
                                    if let Some(index) = build_index_struct(index_node, content) {
                                        let new_index_id = self.get_next_index_id();
                                        let index_ref = IndexRef {
                                            id: IndexId(new_index_id),
                                            class: *class_id,
                                        };
                                        let index_name = index.name.clone();
                                        self.indices.insert(index_name.clone(), index_ref);
                                        new_indices.insert(
                                            index_name.clone(),
                                            (index, index_node.range(), index_ref),
                                        );
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::Projection => {
                                    let projection_node = capture.node;
                                    if let Some(projection) =
                                        build_projection_struct(projection_node, content)
                                    {
                                        let new_projection_id = self.get_next_projection_id();
                                        let projection_ref = ProjectionRef {
                                            id: ProjectionId(new_projection_id),
                                            class: *class_id,
                                        };
                                        let projection_name = projection.name.clone();
                                        self.projections
                                            .insert(projection_name.clone(), projection_ref);
                                        new_projections.insert(
                                            projection_name.clone(),
                                            (projection, projection_node.range(), projection_ref),
                                        );
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::Foreignkey => {
                                    let foreignkey_node = capture.node;
                                    if let Some(foreignkey) =
                                        build_foreignkey_struct(foreignkey_node, content)
                                    {
                                        let new_foreignkey_id = self.get_next_foreignkey_id();
                                        let foreignkey_ref = ForeignKeyRef {
                                            id: ForeignKeyId(new_foreignkey_id),
                                            class: *class_id,
                                        };
                                        let foreignkey_name = foreignkey.name.clone();
                                        self.foreignkeys
                                            .insert(foreignkey_name.clone(), foreignkey_ref);
                                        new_foreignkeys.insert(
                                            foreignkey_name.clone(),
                                            (foreignkey, foreignkey_node.range(), foreignkey_ref),
                                        );
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::Query => {
                                    let query_node = capture.node;
                                    if let Some(query) = build_query_struct(query_node, content) {
                                        let new_query_id = self.get_next_query_id();
                                        let query_ref = QueryRef {
                                            id: QueryId(new_query_id),
                                            class: *class_id,
                                        };
                                        let query_name = query.name.clone();
                                        self.queries.insert(query_name.clone(), query_ref);
                                        new_queries.insert(
                                            query_name.clone(),
                                            (query, query_node.range(), query_ref),
                                        );
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::Xdata => {
                                    let xdata_node = capture.node;
                                    if let Some(xdata) = build_xdata_struct(xdata_node, content) {
                                        let new_xdata_id = self.get_next_xdata_id();
                                        let xdata_ref = XdataRef {
                                            id: XdataId(new_xdata_id),
                                            class: *class_id,
                                        };
                                        let xdata_name = xdata.name.clone();
                                        self.xdata.insert(xdata_name.clone(), xdata_ref);
                                        new_xdata.insert(
                                            xdata_name.clone(),
                                            (xdata, xdata_node.range(), xdata_ref),
                                        );
                                    }
                                    i += 1;
                                    continue;
                                }
                                MemberType::RelativeParameter => {
                                    let parameter_node = capture.node;
                                    if let Some(parameter) =
                                        build_parameter_struct(parameter_node, content)
                                    {
                                        let new_parameter_id = self.get_next_parameter_id();
                                        let parameter_ref = ParameterRef {
                                            id: ParameterId(new_parameter_id),
                                            class: *class_id,
                                        };
                                        self.parameters
                                            .insert(parameter.name.clone(), parameter_ref);
                                        new_parameters.insert(
                                            parameter.name.clone(),
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
                        eprintln!(
                            "error: didn't match type, but node is {:?} and class is {:?}",
                            capture.node, class_name
                        );
                        i += 1;
                        continue;
                    }
                }
            }
        }
        self.inherited_classes = inherited_classes.clone();

        (
            inheritance_changed,
            old_methods,
            new_methods,
            new_properties,
            new_parameters,
            new_relationships,
            new_foreignkeys,
            new_queries,
            new_indices,
            new_triggers,
            new_xdata,
            new_projections,
            new_storage,
            inherited_classes,
            all_methods,
            diagnostics,
        )
    }

    /// Returns the `PublicMethodId` for `method_name`, if this class declares it as public.
    ///
    /// Logs and returns `None` if the method is not present in `public_methods`.
    pub fn get_method_ref(&self, method_name: &str) -> Option<&MethodRef> {
        self.methods.get(method_name)
    }
}
