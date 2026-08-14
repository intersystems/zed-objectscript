use crate::common::{
    find_class_definition, generic_exit_statements, get_member_name_and_range_from_root,
    initial_build_scope_tree, point_to_byte,
};
use crate::config::Config;
use crate::dependency_tracker::{DependencyGraph, Dependents};
use crate::document::Document;
use crate::global_semantic::GlobalSemanticModel;
use crate::local_semantic::LocalSemanticModel;
use crate::override_index::OverrideIndex;
use crate::parse_structures::{
    Class, ClassId, FileType, MethodRef, MethodType, ParameterRef, PropertyRef, RefactorLevel,
    UnresolvedMethodRef, VariableRef,
};
use crate::refactor::{
    refactor_conditionals, refactor_for_statements, refactor_legacy_do_statements,
};
use crate::scope_structures::ScopeId;
use crate::scope_tree::ScopeTree;
use parking_lot::{Mutex, RwLock};
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::OnceLock;
use tower_lsp::lsp_types::Url;
use tree_sitter::{Parser, Point, Range, Tree};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;
use tree_sitter_xml::LANGUAGE_XML;

/// Holds Tree-sitter parsers for each supported ObjectScript file grammar.
pub struct WorkspaceParsers {
    /// Parser for `.mac` / `.inc` routine files.
    pub routine: Mutex<Parser>,
    /// Parser for `.cls` class-definition files.
    pub cls: Mutex<Parser>,
    /// Parser for `.xml` export files.
    pub xml: Mutex<Parser>,
}

impl Debug for WorkspaceParsers {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}

impl WorkspaceParsers {
    /// Construct a `WorkspaceParsers` with ObjectScript grammars initialized.
    ///
    /// - `cls` uses `LANGUAGE_OBJECTSCRIPT_UDL`
    /// - `routine` uses `LANGUAGE_OBJECTSCRIPT_ROUTINE`
    /// - `xml` uses `LANGUAGE_XML`
    ///
    /// Panics if either grammar fails to load (intended to fail-fast during startup).
    pub fn new() -> Self {
        let mut cls_parser = Parser::new();
        cls_parser
            .set_language(&LANGUAGE_OBJECTSCRIPT_UDL.into())
            .expect("Error loading ObjectScript UDL grammar");

        let mut routine_parser = Parser::new();
        routine_parser
            .set_language(&LANGUAGE_OBJECTSCRIPT_ROUTINE.into())
            .expect("Error loading ObjectScript routine grammar");

        let mut xml_parser = Parser::new();
        xml_parser
            .set_language(&LANGUAGE_XML.into())
            .expect("Error loading XML grammar");

        Self {
            routine: Mutex::new(routine_parser),
            cls: Mutex::new(cls_parser),
            xml: Mutex::new(xml_parser),
        }
    }
}

/// Stores all workspace-wide state needed to serve LSP features.
///
/// `ProjectData` is the in-memory “database” for a single workspace: it owns the
/// current configuration, parsed documents, semantic models, and symbol indexes
/// used for lookups like go-to-definition, references, and override resolution.
///
#[derive(Debug)]
pub struct ProjectData {
    /// Stores the User Settings for this Workspace.
    pub config: Config,
    /// Maps Url -> Document for each `.cls`, `.mac`, and `.inc` document in this Workspace.
    pub documents: HashMap<Url, Document>,
    /// Stores all semantic information for this Workspace.
    pub global_semantic_model: GlobalSemanticModel,
    /// Maps class name -> ClassId(index) for each class in this workspace.
    pub classes: HashMap<String, ClassId>,
    /// Maps Class Name -> another hashmap which maps Method Name -> MethodRef for all Methods Accessible from the class.
    pub method_defs: HashMap<String, HashMap<String, MethodRef>>,
    /// Maps Class Name -> another hashmap which maps Property Name -> PropertyRef for all Properties Accessible from the class.
    pub property_defs: HashMap<String, HashMap<String, PropertyRef>>,
    /// Maps Class Name -> another hashmap which maps Parameter Name -> ParameterRef for all Parameters Accessible from the class.
    pub parameter_defs: HashMap<String, HashMap<String, ParameterRef>>,
    /// Maps Var Name -> another hashmap which maps MethodRef -> HashMap of ScopeId -> Vec<VariableRef> for that variable.
    pub pub_var_defs: HashMap<String, HashMap<MethodRef, HashMap<ScopeId, Vec<VariableRef>>>>,
    /// Holds the OverrideIndex for the workspace.
    pub override_index: OverrideIndex,
    /// Reverse inheritance index used by hierarchy-aware public variable lookup.
    pub dependent_class_index: Dependents,
    /// Graph of all calls to methods/procedures/subroutines for each class
    pub dependency_graph: DependencyGraph,
    /// Unresolved Class Name -> Class Id that tried to inherit it
    pub unresolved_inheritance_references: HashMap<String, HashSet<ClassId>>,
    /// (Unresolved ClassName, Unresolved Method Name) -> HashSet<(MethodRef, Range)> representing the place the unresolved reference took place.
    pub unresolved_method_references: HashMap<(String, String), HashSet<(MethodRef, Range)>>,
}

/// Concurrency wrapper for a workspace’s state and parsers.
///
/// `ProjectState` holds the project root path and a lock-protected `ProjectData`,
/// along with Tree-sitter parsers shared across requests. This is the primary
/// entry point for workspace-level operations (open/update/index).
#[derive(Debug)]
pub struct ProjectState {
    /// Workspace root path (set once during initialize()).
    pub project_root_path: OnceLock<Option<PathBuf>>,
    /// Lock-protected workspace data (documents, semantics, symbols, indexes).
    pub data: RwLock<ProjectData>,
    /// Reusable parsers for `.cls` and routine files.
    pub parsers: WorkspaceParsers,
}

impl ProjectData {
    /// Return basic immutable snapshot information for a document.
    ///
    /// Produces `(file_type, content, version, tree)` for the document at `url`. The text and tree
    /// are cloned so callers can use them without holding a borrow on `ProjectData`.
    ///
    /// Returns `None` if the document is not currently tracked.
    pub fn get_document_info(&self, url: &Url) -> Option<(FileType, String, i32, Tree)> {
        let Some(document) = self.get_document(url) else {
            generic_exit_statements("ProjectData", "get_document_info");
            return None;
        };
        let curr_version = document.version.unwrap_or(0);
        let current_text = document.content.clone();
        let curr_tree = document.tree.clone();
        Some((
            document.file_type.clone(),
            current_text,
            curr_version,
            curr_tree,
        ))
    }

    /// Add a document only if it is not already present.
    /// Returns true if the document was present, false otherwise.
    pub fn add_document_if_absent(
        &mut self,
        url: Url,
        code: String,
        tree: &Tree,
        filetype: FileType,
        class_name: String,
        class_range: Range,
        version: Option<i32>,
    ) -> bool {
        if self.documents.contains_key(&url) {
            eprintln!("Document already exists for file at :{:?}", url.path());
            return true;
        }
        let class_id = if filetype == FileType::Xml {
            None
        } else {
            Some(ClassId(self.global_semantic_model.next_id()))
        };
        self.add_document(
            url,
            code.as_str(),
            tree,
            filetype,
            class_id,
            class_name,
            version,
            class_range,
        );
        false
    }

    /// Refactor a document. Refactoring options are in `RefactorLevel`, and include
    /// `Refactor Legacy Dotted Do Statements`, `Refactor Legacy If/Else Statements`
    /// `Refactor Legacy For Statements`, or do all three actions.
    pub fn refactor_document(&self, url: &Url, refactor_level: RefactorLevel) -> Option<String> {
        let (filetype, content) = {
            let Some(document) = self.get_document(url) else {
                eprintln!(
                    "Tried to refactor document {:?}, but it does not exist ",
                    url.path()
                );
                return None;
            };
            (document.file_type.clone(), document.content.clone())
        };
        if filetype == FileType::Xml {
            return None;
        }
        let updated_content = match refactor_level {
            RefactorLevel::DoCommands => {
                if filetype != FileType::Routine {
                    return None;
                }
                refactor_legacy_do_statements(content.as_str())
            }
            RefactorLevel::Conditionals => {
                refactor_conditionals(content.as_str(), filetype.clone())
            }
            RefactorLevel::ForCommands => {
                refactor_for_statements(content.as_str(), filetype.clone())
            }
            RefactorLevel::All => {
                let content_after_do_refactor = if filetype == FileType::Routine {
                    refactor_legacy_do_statements(content.as_str())
                } else {
                    content.clone()
                };
                let content_after_if_refactor =
                    refactor_conditionals(content_after_do_refactor.as_str(), filetype.clone());
                refactor_for_statements(content_after_if_refactor.as_str(), filetype.clone())
            }
        };
        if content == updated_content.as_str() {
            None
        } else {
            Some(updated_content)
        }
    }

    /// Refactor all ObjectScript documents in this workspace. Refactoring options are in `RefactorLevel`, and include
    /// `Refactor Legacy Dotted Do Statements`, `Refactor Legacy If/Else Statements`
    /// `Refactor Legacy For Statements`, or do all three actions.
    pub fn refactor(&self, refactor_level: RefactorLevel) -> Vec<(String, Url)> {
        let mut changed = Vec::new();
        let urls: Vec<&Url>;
        if refactor_level == RefactorLevel::DoCommands {
            urls = self
                .documents
                .iter()
                .filter_map(|(url, document)| {
                    (document.file_type == FileType::Routine).then_some(url)
                })
                .collect();
        } else {
            urls = self.documents.keys().collect();
        }
        for url in urls {
            let Some(refactored) = self.refactor_document(url, refactor_level) else {
                continue;
            };
            changed.push((refactored, url.clone()));
        }
        changed
    }

    fn resolve_method_references(
        &mut self,
        unresolved_method_refs: &HashSet<UnresolvedMethodRef>,
        method_ref: MethodRef,
        class_id: ClassId,
    ) {
        for unresolved_method_ref in unresolved_method_refs {
            let referenced_method_in_class = self
                .global_semantic_model
                .get_class(&class_id)
                .and_then(|c| c.methods.get(&unresolved_method_ref.method).copied());
            if let Some(referenced_method) = self
                .method_defs
                .get(&unresolved_method_ref.class)
                .and_then(|methods| methods.get(&unresolved_method_ref.method))
            {
                self.dependency_graph.add_edge(
                    method_ref,
                    *referenced_method,
                    unresolved_method_ref.method_call_range,
                );
            } else if let Some(referenced_method) = referenced_method_in_class {
                self.dependency_graph.add_edge(
                    method_ref,
                    referenced_method,
                    unresolved_method_ref.method_call_range,
                );
            } else {
                self.unresolved_method_references
                    .entry((
                        unresolved_method_ref.class.clone(),
                        unresolved_method_ref.method.clone(),
                    ))
                    .or_insert(HashSet::new())
                    .insert((method_ref, unresolved_method_ref.method_call_range));
            }
        }
    }

    /// Parse and register a new document, initializing semantic + symbol state for `.cls` files.
    ///
    /// For all ObjectScript files, this:
    /// - Extracts the class range
    /// - Builds an initial `Class` and method list from the tree-sitter tree
    /// - Creates a `ClassGlobalSymbol`, `ScopeTree`, and `Document`
    /// - Adds public methods into the global semantic model and method symbol tables
    /// - Adds private methods into the local semantic model and scope tree symbols
    /// - Registers class ids and local semantic model ids for later rebuilds
    pub fn add_document(
        &mut self,
        url: Url,
        content: &str,
        tree: &Tree,
        filetype: FileType,
        class_id: Option<ClassId>,
        class_name: String,
        version: Option<i32>,
        class_range: Range,
    ) {
        if filetype == FileType::Xml {
            let document = Document::new(
                content.to_string(),
                tree.clone(),
                filetype,
                "XML".to_string(),
                None,
                ScopeTree::new(None),
                version,
            );
            self.documents.insert(url, document);
            return;
        } else if filetype == FileType::Routine || filetype == FileType::Cls {
            let Some(class_id) = class_id else {
                return;
            };
            if let Some(_) = self.get_document(&url) {
                eprintln!("Error: Document already exists");
                return;
            }
            let is_rtn = if filetype == FileType::Routine {
                true
            } else {
                false
            };
            let scope_tree = initial_build_scope_tree(&tree, class_id, content, is_rtn);
            let mut document = Document::new(
                content.to_string(),
                tree.clone(),
                filetype,
                class_name.clone(),
                Some(class_id),
                scope_tree,
                version,
            );
            let local_semantic_model = LocalSemanticModel::new();
            self.global_semantic_model
                .new_local_semantic(class_id, local_semantic_model);
            let mut class = Class::new(class_name.clone(), is_rtn);
            self.classes.insert(class_name.clone(), class_id);
            let starting_node = if is_rtn {
                tree.root_node()
            } else {
                let Some(node) = find_class_definition(tree.root_node()) else {
                    eprintln!(
                        "Error: Failed to find class definition for class named {:?}",
                        class_name
                    );
                    return;
                };
                node
            };
            // this is a new class, so some things returned from this function are not applicable
            let (_, _, _, methods, properties, parameters, inherited_classes, _, _, _) = class
                .build_class(
                    starting_node,
                    content,
                    is_rtn,
                    &class_id,
                    class_range,
                    &class_name,
                );

            class.build_imports(tree, content);

            // adds class and class symbol to global semantic model
            self.global_semantic_model
                .new_class(class, class_id, class_range, url.clone());
            // inherits is_procedure_block, is_final, language from leftmost inherited class if applicable
            self.rebuild_keyword_inheritance_for_class(&class_id);
            // NOTE: this must be checked after the keyword inheritance is completed.
            let (class_is_final, class_is_procedure_block) = {
                if let Some(class) = self.global_semantic_model.get_class(&class_id) {
                    (class.is_final, class.is_procedure_block)
                } else {
                    return;
                }
            };
            let mut classes_to_recompute_inheritance = HashSet::new();
            self.new_class_inheritance(
                &class_name,
                class_id,
                &inherited_classes,
                class_is_final.unwrap_or(false),
                &mut classes_to_recompute_inheritance,
            );
            classes_to_recompute_inheritance.insert(class_id);
            let mut unresolved_orefs = HashMap::new();
            // class id dne yet, because it gets added after. instead, we can just create the method ids here
            for (method_name, (mut method, method_range, method_ref, public_variables_declared)) in
                methods
            {
                self.resolve_unresolved_method(
                    &(class_name.clone(), method_name.clone()),
                    method_ref,
                );
                self.dependency_graph.get_or_add_node(method_ref);
                let method_type = method.method_type.clone();
                let mut variable_info = Vec::new();
                let mut unresolved_method_refs = HashSet::new();
                let mut unresolved_oref_method_refs = HashSet::new();
                match method_type {
                    MethodType::ClassMethod | MethodType::InstanceMethod => {
                        if let Some(method_definition_node) =
                            tree.root_node().named_descendant_for_byte_range(
                                method_range.start_byte,
                                method_range.end_byte,
                            )
                        {
                            (
                                _,
                                _,
                                variable_info,
                                unresolved_method_refs,
                                unresolved_oref_method_refs,
                            ) = method.rebuild_method(
                                method_definition_node,
                                content,
                                &document.scope_tree,
                                method_type,
                                method_range,
                                public_variables_declared,
                                class_is_final,
                                None,
                                class_is_procedure_block,
                                &class_name,
                            );
                        }
                    }
                    MethodType::Procedure(_) => {
                        if let Some(method_definition_node) =
                            tree.root_node().named_descendant_for_byte_range(
                                method_range.start_byte,
                                method_range.end_byte,
                            )
                        {
                            (
                                _,
                                _,
                                variable_info,
                                unresolved_method_refs,
                                unresolved_oref_method_refs,
                            ) = method.rebuild_method(
                                method_definition_node,
                                content,
                                &document.scope_tree,
                                method_type,
                                method_range,
                                public_variables_declared,
                                class_is_final,
                                None,
                                class_is_procedure_block,
                                &class_name,
                            );
                        }
                    }
                    MethodType::Subroutine(_) | MethodType::Routine => {
                        (
                            _,
                            _,
                            variable_info,
                            unresolved_method_refs,
                            unresolved_oref_method_refs,
                        ) = method.rebuild_method(
                            tree.root_node(),
                            content,
                            &document.scope_tree,
                            method_type,
                            method_range,
                            public_variables_declared,
                            class_is_final,
                            None,
                            class_is_procedure_block,
                            &class_name,
                        );
                    }
                }
                for (variable, variable_range, variable_dependencies, variable_scope_id) in
                    variable_info
                {
                    let variable_name = variable.name.clone();
                    let variable_is_public = variable.is_public;
                    // add it to global semantic model (if public) or local semantic model/scope tree (if private)
                    // global semantic will add it to local semantic if private
                    let variable_ref = self.global_semantic_model.new_variable(
                        variable,
                        method_ref,
                        variable_scope_id,
                        variable_dependencies.clone(),
                        variable_range,
                        url.clone(),
                    );

                    // add variable ref and corresponding scope id to method
                    method
                        .variables
                        .entry(variable_name.clone())
                        .or_insert(Vec::new())
                        .push((variable_ref, variable_scope_id));

                    if variable_is_public {
                        document.scope_tree.new_public_var_symbol(
                            variable_name.clone(),
                            variable_range,
                            variable_ref,
                        );
                        self.pub_var_defs
                            .entry(variable_name)
                            .or_insert(HashMap::new())
                            .entry(method_ref)
                            .or_insert(HashMap::new())
                            .entry(variable_scope_id)
                            .or_insert(Vec::new())
                            .push(variable_ref);
                    } else {
                        document.scope_tree.new_variable_symbol(
                            variable_name,
                            variable_range,
                            variable_dependencies,
                            variable_ref,
                        );
                    }
                }
                self.dependency_graph.get_or_add_node(method_ref);
                self.method_defs
                    .entry(class_name.clone())
                    .or_insert_with(HashMap::new)
                    .insert(method_name.clone(), method_ref);
                self.resolve_method_references(&unresolved_method_refs, method_ref, class_id);
                unresolved_orefs.insert(method_ref, unresolved_oref_method_refs);
                if !method.is_public {
                    // creates method symbol in scope tree (private)
                    document.scope_tree.new_method_symbol(
                        method_name.clone(),
                        method_range,
                        method_ref,
                        url.clone(),
                    );
                }
                // adds method to global semantic model if public, and to local semantic model if private
                // also, creates method symbol if public
                self.global_semantic_model.new_method(
                    method,
                    method_ref,
                    method_range,
                    url.clone(),
                );
                self.compute_inheritance_override_index_method(
                    &classes_to_recompute_inheritance,
                    &HashSet::new(),
                    method_name,
                    method_ref,
                    true,
                    class_is_final.unwrap_or(false),
                );
            }

            for (method_ref, unresolved_oref_methods) in unresolved_orefs {
                for (oref_name, oref_method_name, method_call_range, current_method_name) in
                    unresolved_oref_methods
                {
                    let (resolved, unresolved) = self.resolve_oref_methods(
                        method_ref,
                        &oref_name,
                        &oref_method_name,
                        method_call_range,
                        &current_method_name,
                        &document.scope_tree,
                    );
                    for (key, value) in unresolved {
                        self.unresolved_method_references
                            .entry(key)
                            .or_insert(HashSet::new())
                            .extend(value);
                    }
                    for referenced_method_ref in &resolved {
                        self.dependency_graph.add_edge(
                            method_ref,
                            *referenced_method_ref,
                            method_call_range,
                        );
                    }
                }
            }
            for (property_name, (property, property_range, property_ref)) in properties {
                if !property.is_public {
                    // creates and stores property symbol in scope tree
                    document.scope_tree.new_property_symbol(
                        property_name.clone(),
                        property_range,
                        property_ref,
                        url.clone(),
                    )
                }
                // adds property to gsm if public, lsm if private
                // if public, creates and stores property symbol in gsm
                self.global_semantic_model.new_property(
                    property,
                    property_ref,
                    property_range,
                    url.clone(),
                );
                self.property_defs
                    .entry(class_name.clone())
                    .or_insert_with(HashMap::new)
                    .insert(property_name.clone(), property_ref);
                self.compute_inheritance_override_index_property(
                    &classes_to_recompute_inheritance,
                    &HashSet::new(),
                    property_name,
                    property_ref,
                    true,
                    class_is_final.unwrap_or(false),
                );
            }
            for (parameter_name, (parameter, parameter_range, parameter_ref)) in parameters {
                // creates property symbol and stores it in global semantic model
                // also stores parameter in gsm
                self.global_semantic_model.new_parameter(
                    parameter,
                    parameter_ref,
                    parameter_range,
                    url.clone(),
                );
                // add property ref to workspace
                self.parameter_defs
                    .entry(class_name.clone())
                    .or_insert_with(HashMap::new)
                    .insert(parameter_name.clone(), parameter_ref);
                self.compute_inheritance_override_index_parameter(
                    &classes_to_recompute_inheritance,
                    &HashSet::new(),
                    parameter_name,
                    parameter_ref,
                    true,
                    class_is_final.unwrap_or(false),
                );
            }
            self.documents.insert(url.clone(), document);
        }
    }

    fn remove_stale_class_from_dependent_classes(
        &mut self,
        class_id: ClassId,
        inherited_classes: &Vec<String>,
    ) -> Vec<ClassId> {
        let mut stale_classes = Vec::new();
        for old_inherited_class in inherited_classes {
            if let Some(inherited_class_id) = self.classes.get(old_inherited_class) {
                stale_classes.push(*inherited_class_id);
                if let Some(inherited_class_dependents) = self
                    .dependent_class_index
                    .direct_subclasses
                    .get_mut(inherited_class_id)
                {
                    inherited_class_dependents.remove(&class_id);
                }
                self.dependent_class_index
                    .rebuild_transitive_subclasses(*inherited_class_id);
            }
        }
        stale_classes
    }

    /// For each `String` in `inherited_classes` representing the class name, this
    /// finds the corresponding `ClassId` and adds `dependent_class_id` and all dependents of `dependent_class_id` to the newly inherited classes.
    fn add_dependent_class_to_inherited_class(
        &mut self,
        dependent_class_id: ClassId,
        inherited_classes: &Vec<String>,
        classes_to_recompute_inheritance: &mut HashSet<ClassId>,
    ) {
        for inherited_cls_name in inherited_classes {
            if let Some(inherited_class_id) = self.classes.get(inherited_cls_name).copied() {
                self.dependent_class_index
                    .direct_subclasses
                    .entry(inherited_class_id)
                    .or_insert(HashSet::new())
                    .insert(dependent_class_id);
                self.dependent_class_index
                    .dependent_classes
                    .entry(inherited_class_id)
                    .or_insert(HashSet::new())
                    .insert(dependent_class_id);
                let new_class_dependents = self
                    .dependent_class_index
                    .dependent_classes
                    .get(&dependent_class_id)
                    .cloned()
                    .unwrap_or_default();
                self.dependent_class_index
                    .dependent_classes
                    .entry(inherited_class_id)
                    .or_insert(HashSet::new())
                    .extend(new_class_dependents.clone());
                classes_to_recompute_inheritance.extend(new_class_dependents);
            } else {
                self.unresolved_inheritance_references
                    .entry(inherited_cls_name.clone())
                    .or_insert(HashSet::new())
                    .insert(dependent_class_id);
            }
        }
    }

    fn resolve_unresolved_class(
        &mut self,
        unresolved_class_name: &str,
        classes_to_recompute_inheritance: &mut HashSet<ClassId>,
    ) {
        // find any current classes that already extend this class
        if let Some(classes_already_extending_current_class) = self
            .unresolved_inheritance_references
            .get(unresolved_class_name)
            .cloned()
        {
            let inherited_class = &vec![unresolved_class_name.to_string()];
            for dependent_class_id in classes_already_extending_current_class {
                // inheritance class id represents
                self.add_dependent_class_to_inherited_class(
                    dependent_class_id,
                    inherited_class,
                    classes_to_recompute_inheritance,
                );
                classes_to_recompute_inheritance.insert(dependent_class_id);
            }
        }
        self.unresolved_inheritance_references
            .remove(unresolved_class_name);
    }

    fn resolve_unresolved_method(&mut self, key: &(String, String), method_ref: MethodRef) {
        if let Some(unresolved_method_callers) = self.unresolved_method_references.remove(key) {
            for method_caller in unresolved_method_callers {
                self.dependency_graph
                    .add_edge(method_caller.0, method_ref, method_caller.1);
            }
        }
    }

    fn update_class_name_in_workspace(&mut self, old_class_name: &str, new_class_name: &str) {
        // remove all pointers to the old class name
        self.classes.remove(old_class_name);
        self.parameter_defs.remove(old_class_name);
        self.property_defs.remove(old_class_name);
        // move all method refs stored in old class hash to new class
        if let Some(method_hash) = self.method_defs.remove(old_class_name) {
            self.method_defs
                .insert(new_class_name.to_string(), method_hash);
        }
    }

    /// Adds the dependent classes to the hashset of `classids` to rebuild in override index.
    /// Resolves any previous unresolved inheritance efforts for `class_id`.
    fn new_class_inheritance(
        &mut self,
        new_class_name: &str,
        class_id: ClassId,
        new_inherited_classes: &Vec<String>,
        new_class_is_final: bool,
        classes_to_recompute_inheritance: &mut HashSet<ClassId>,
    ) {
        if !new_class_is_final {
            self.resolve_unresolved_class(new_class_name, classes_to_recompute_inheritance);
        }
        self.add_dependent_class_to_inherited_class(
            class_id,
            new_inherited_classes,
            classes_to_recompute_inheritance,
        );
        let mut curr_class_hash = HashSet::new();
        curr_class_hash.insert(class_id);
        self.build_override_index_for_classes(&curr_class_hash);
    }

    fn update_class_inheritance(
        &mut self,
        old_class_name: &str,
        new_class_name: &str,
        class_id: ClassId,
        old_inherited_classes: &Vec<String>,
        new_inherited_classes: &Vec<String>,
        inheritance_changed: bool,
        old_class_is_final: bool,
        new_class_is_final: bool,
        classes_to_recompute_inheritance: &mut HashSet<ClassId>,
    ) {
        // (bool) recompute inheritance for ALL class members for the subclasses
        // resolve any references from other classes -> the new class name
        if !new_class_is_final {
            self.resolve_unresolved_class(new_class_name, classes_to_recompute_inheritance);
        }
        let class_name_changed = new_class_name != old_class_name;
        let is_final_changed = old_class_is_final != new_class_is_final;
        if (class_name_changed || is_final_changed || inheritance_changed)
            && let Some(current_dependents) =
                self.dependent_class_index.dependent_classes.get(&class_id)
        {
            classes_to_recompute_inheritance.extend(current_dependents);

            if new_class_is_final || class_name_changed {
                // remove all references to the old class
                self.unresolved_inheritance_references
                    .entry(old_class_name.to_string())
                    .or_insert(HashSet::new())
                    .extend(current_dependents);
                self.dependent_class_index
                    .dependent_classes
                    .remove(&class_id);
                self.dependent_class_index
                    .direct_subclasses
                    .remove(&class_id);
            }
        }
        let mut curr_class_hash = HashSet::new();
        curr_class_hash.insert(class_id);
        if !new_class_is_final {
            if let Some(dependents) = self.dependent_class_index.direct_subclasses.get(&class_id) {
                curr_class_hash.extend(dependents)
            }
        }
        if inheritance_changed {
            self.remove_stale_class_from_dependent_classes(class_id, old_inherited_classes);
            self.add_dependent_class_to_inherited_class(
                class_id,
                new_inherited_classes,
                classes_to_recompute_inheritance,
            );
            self.build_override_index_for_classes(&curr_class_hash);
        }
        if class_name_changed {
            // remove incoming edges from other classes in the dependency graph
            let old_method_nodes = self.dependency_graph.get_class_nodes(&class_id);
            // remove edges to all method refs of this class (unless they are from a method within this class)
            for old_node_index in old_method_nodes {
                let method_caller_refs = self
                    .dependency_graph
                    .remove_direct_ancestors(old_node_index, &curr_class_hash);
                // all old refs to the old class name (and any of its methods) are now unresolved if they aren't from a method in the same class
                if let Some(stale_method_ref) = self
                    .dependency_graph
                    .get_method_ref_from_node_index(old_node_index)
                    && let Some(old_method) =
                        self.global_semantic_model.get_method(stale_method_ref)
                {
                    let old_method_name = old_method.name.clone();
                    let old_method_is_final = old_method.is_final;
                    self.unresolved_method_references
                        .entry((old_class_name.to_string(), old_method_name.clone()))
                        .or_insert(HashSet::new())
                        .extend(method_caller_refs);
                    if let Some(new_class) = self.global_semantic_model.get_class(&class_id)
                        && let Some(new_method_ref) = new_class.get_method_ref(&old_method_name)
                        && !old_method_is_final.unwrap_or(new_class.is_final.unwrap_or(false))
                    {
                        self.resolve_unresolved_method(
                            &(new_class_name.to_string(), old_method_name),
                            *new_method_ref,
                        );
                    }
                }
            }
            self.update_class_name_in_workspace(old_class_name, new_class_name);
        }
    }

    fn compute_inheritance_override_index_parameter(
        &mut self,
        classes_to_fully_recompute_inheritance: &HashSet<ClassId>,
        subclasses_to_recompute_inheritance: &HashSet<ClassId>,
        parameter_name: String,
        parameter_ref: ParameterRef,
        new_parameter: bool,
        class_is_final: bool,
    ) {
        if !classes_to_fully_recompute_inheritance.is_empty()
            && let Some(parameter) = self.global_semantic_model.get_parameter(&parameter_ref)
            && (!parameter.is_final.unwrap_or(class_is_final) || !new_parameter)
        {
            let extended_parameters = self.build_override_index_for_parameter(
                &classes_to_fully_recompute_inheritance,
                &parameter_name,
            );
            for (extended_class_name, parameter_ref_map) in extended_parameters {
                self.parameter_defs
                    .entry(extended_class_name.clone())
                    .or_insert(HashMap::new())
                    .extend(parameter_ref_map);
            }
        }
        if !subclasses_to_recompute_inheritance.is_empty() {
            let extended_parameters = self.build_override_index_for_parameter(
                &subclasses_to_recompute_inheritance,
                &parameter_name,
            );
            for (extended_class_name, parameter_ref_map) in extended_parameters {
                self.parameter_defs
                    .entry(extended_class_name.clone())
                    .or_insert(HashMap::new())
                    .extend(parameter_ref_map);
            }
        }
    }

    fn compute_inheritance_override_index_property(
        &mut self,
        classes_to_fully_recompute_inheritance: &HashSet<ClassId>,
        subclasses_to_recompute_inheritance: &HashSet<ClassId>,
        property_name: String,
        property_ref: PropertyRef,
        new_property: bool,
        class_is_final: bool,
    ) {
        if !classes_to_fully_recompute_inheritance.is_empty()
            && let Some(property) = self.global_semantic_model.get_property(&property_ref)
            && (!property.is_final.unwrap_or(class_is_final) || !new_property)
        {
            let extended_properties = self.build_override_index_for_property(
                &classes_to_fully_recompute_inheritance,
                &property_name,
            );
            for (extended_class_name, property_ref_map) in extended_properties {
                self.property_defs
                    .entry(extended_class_name.clone())
                    .or_insert(HashMap::new())
                    .extend(property_ref_map);
            }
        }
        if !subclasses_to_recompute_inheritance.is_empty() {
            let extended_properties = self.build_override_index_for_property(
                &subclasses_to_recompute_inheritance,
                &property_name,
            );
            for (extended_class_name, property_ref_map) in extended_properties {
                self.property_defs
                    .entry(extended_class_name.clone())
                    .or_insert(HashMap::new())
                    .extend(property_ref_map);
            }
        }
    }

    fn compute_inheritance_override_index_method(
        &mut self,
        classes_to_fully_recompute_inheritance: &HashSet<ClassId>,
        subclasses_to_recompute_inheritance: &HashSet<ClassId>,
        method_name: String,
        method_ref: MethodRef,
        new_method: bool,
        class_is_final: bool,
    ) {
        if !classes_to_fully_recompute_inheritance.is_empty()
            && let Some(method) = self.global_semantic_model.get_method(&method_ref)
            && (!method.is_final.unwrap_or(class_is_final) || !new_method)
        {
            let extended_methods = self.build_override_index_for_method(
                &classes_to_fully_recompute_inheritance,
                &method_name,
            );
            for (extended_class_name, method_ref_map) in extended_methods {
                self.method_defs
                    .entry(extended_class_name.clone())
                    .or_insert(HashMap::new())
                    .extend(method_ref_map);
            }
        }
        if !subclasses_to_recompute_inheritance.is_empty() {
            let extended_methods = self.build_override_index_for_method(
                &subclasses_to_recompute_inheritance,
                &method_name,
            );
            for (extended_class_name, method_ref_map) in extended_methods {
                self.method_defs
                    .entry(extended_class_name.clone())
                    .or_insert(HashMap::new())
                    .extend(method_ref_map);
            }
        }
    }

    fn remove_stale_methods(
        &mut self,
        stale_methods: &HashSet<String>,
        class_name: &str,
        class_name_changed: bool,
        classes_to_fully_recompute_inheritance: &HashSet<ClassId>,
        subclasses_to_recompute_inheritance: &HashSet<ClassId>,
        new_class_is_final: bool,
    ) -> HashSet<MethodRef> {
        let mut stale_method_refs = HashSet::new();
        // first remove all stale members
        for stale_method in stale_methods {
            // remove method ref
            if let Some(stale_method_ref) = self
                .method_defs
                .get_mut(class_name)
                .and_then(|methods| methods.remove(stale_method))
            {
                if !class_name_changed
                    && let Some(stale_node_index) = self.dependency_graph.get_node(stale_method_ref)
                {
                    let method_caller_refs = self
                        .dependency_graph
                        .remove_incoming_calls_to_node(*stale_node_index);
                    if let Some(old_method) =
                        self.global_semantic_model.get_method(&stale_method_ref)
                    {
                        let old_method_name = old_method.name.clone();
                        self.unresolved_method_references
                            .entry((class_name.to_string(), old_method_name.clone()))
                            .or_insert(HashSet::new())
                            .extend(method_caller_refs);
                        self.compute_inheritance_override_index_method(
                            classes_to_fully_recompute_inheritance,
                            subclasses_to_recompute_inheritance,
                            old_method_name,
                            stale_method_ref,
                            false,
                            new_class_is_final,
                        );
                    }
                }
                self.global_semantic_model.remove_method(&stale_method_ref);
                stale_method_refs.insert(stale_method_ref);
                for method_map in self.pub_var_defs.values_mut() {
                    method_map.remove(&stale_method_ref);
                }
            }
        }
        stale_method_refs
    }

    /// Returns true if successful, false otherwise
    pub fn incremental_update_document(
        &mut self,
        url: Url,
        tree: &Tree,
        file_type: FileType,
        version: i32,
        content: &str,
        changed_ranges: Vec<Range>,
        new_class_name: String,
        new_class_range: Range,
    ) -> bool {
        if file_type == FileType::Xml {
            let Some(document) = self.get_document_mut(&url) else {
                generic_exit_statements("Error: document DNE for path: {:?}", url.path());
                self.add_document(
                    url,
                    content,
                    tree,
                    file_type,
                    None,
                    new_class_name,
                    Some(version),
                    new_class_range,
                );
                return true;
            };
            document.version = Some(version);
            document.file_type = file_type;
            document.tree = tree.clone();
            document.content = content.to_string();
            document.class_name = new_class_name;
            document.class_id = None;
            document.scope_tree = ScopeTree::new(None);
            return true;
        } else if file_type == FileType::Routine || file_type == FileType::Cls {
            let is_rtn = if file_type == FileType::Routine {
                true
            } else {
                false
            };

            if let Some(document) = self.get_document_mut(&url) {
                document.class_name = new_class_name.clone();
                document.file_type = file_type;
                document.content = content.to_string();
                document.tree = tree.clone();
                document.version = Some(version);
            }

            let (old_class_id, old_class_name, old_is_final, old_inherited_classes, old_scope_tree) = {
                let Some(doc) = self.get_document(&url) else {
                    eprintln!(
                        "Error: Document for url {:?} DNE aborting update_document",
                        url.path()
                    );
                    return false;
                };
                let Some(cls_id) = doc.class_id else {
                    eprintln!(
                        "Error: Class ID for document {:?} DNE aborting update_document",
                        doc
                    );
                    return false;
                };
                let old_member_name = doc.class_name.clone();
                let Some(class) = self.global_semantic_model.get_class(&cls_id) else {
                    return false;
                };

                (
                    cls_id,
                    old_member_name,
                    class.is_final.clone(),
                    class.inherited_classes.clone(),
                    doc.scope_tree.clone(),
                )
            };

            // this updates all the class members in the class itself, and then use the
            // returned results to update the global semantic model/ local semantic model/ scope tree
            let (
                inheritance_changed,
                class_name_changed,
                stale_methods,
                new_methods,
                properties_already_rebuilt,
                parameters_already_rebuilt,
                new_inherited_classes,
                all_methods,
                stale_properties,
                stale_parameters,
            ) = {
                let Some(class) = self.global_semantic_model.get_mut_class(&old_class_id) else {
                    return false;
                };
                class.build_imports(tree, content);
                class.build_class(
                    tree.root_node(),
                    content,
                    is_rtn,
                    &old_class_id,
                    new_class_range,
                    &new_class_name,
                )
            };
            // update class symbol and class ref
            if let Some(class_symbol) = self
                .global_semantic_model
                .get_class_symbol_mut(&old_class_id)
            {
                class_symbol.name = new_class_name.clone();
                class_symbol.location = new_class_range;
                class_symbol.alive = true;
            }

            self.classes.insert(new_class_name.clone(), old_class_id);

            // rebuild scope tree
            let mut scope_tree = initial_build_scope_tree(&tree, old_class_id, content, is_rtn);
            // copy over the old variable defs from the old scope tree into the new rebuilt scope tree
            let old_class_member_scopes = old_scope_tree.get_root_children_scopes();
            for old_scope in old_class_member_scopes {
                scope_tree.copy_method_scope(&old_scope, &old_scope_tree);
            }
            scope_tree.private_method_defs = old_scope_tree.private_method_defs;

            // rebuild keywords for class (is_procedure, is_final, language)
            self.rebuild_keyword_inheritance_for_class(&old_class_id);
            // NOTE: this must be checked after the keyword inheritance is completed.
            let (new_class_is_final, new_class_is_procedure_block) = {
                if let Some(class) = self.global_semantic_model.get_class(&old_class_id) {
                    (class.is_final, class.is_procedure_block)
                } else {
                    return false;
                }
            };
            // classes to recompute inheritance includes all classes that
            // the override index should be rebuilt for
            let mut classes_to_fully_recompute_inheritance = HashSet::new();
            let mut methods_already_rebuilt = HashSet::new();
            self.update_class_inheritance(
                &old_class_name,
                &new_class_name,
                old_class_id,
                &old_inherited_classes,
                &new_inherited_classes,
                inheritance_changed,
                old_is_final.unwrap_or(false),
                new_class_is_final.unwrap_or(false),
                &mut classes_to_fully_recompute_inheritance,
            );
            // this consists of all subclasses that are NOT in classes_to_fully_recompute_inheritance
            let subclasses_to_recompute_inheritance: HashSet<ClassId> = self
                .dependent_class_index
                .dependent_classes
                .get(&old_class_id)
                .unwrap_or(&HashSet::new())
                .difference(&classes_to_fully_recompute_inheritance)
                .cloned()
                .collect();

            let stale_method_refs = self.remove_stale_methods(
                &stale_methods,
                &new_class_name,
                class_name_changed,
                &classes_to_fully_recompute_inheritance,
                &subclasses_to_recompute_inheritance,
                new_class_is_final.unwrap_or(false),
            );

            // remove all property_defs and parameter defs for the class (will be rebuilt fully)
            self.property_defs.remove(&new_class_name);
            self.parameter_defs.remove(&new_class_name);
            // remove stale methods, properties, and parameters in gsm and lsm
            self.global_semantic_model.incremental_reset_doc_semantics(
                &old_class_id,
                stale_method_refs,
                stale_properties,
                stale_parameters,
            );

            let mut unresolved_orefs = HashMap::new();

            for (method_name, (mut method, method_range, method_ref, public_variables_declared)) in
                new_methods
            {
                let mut unresolved_method_refs = HashSet::new();
                let mut unresolved_oref_method_refs = HashSet::new();
                let mut variable_info = Vec::new();
                methods_already_rebuilt.insert(method_name.clone());
                self.dependency_graph.get_or_add_node(method_ref);
                let method_type = method.method_type.clone();
                match method_type {
                    MethodType::ClassMethod | MethodType::InstanceMethod => {
                        if let Some(method_definition_node) =
                            tree.root_node().named_descendant_for_byte_range(
                                method_range.start_byte,
                                method_range.end_byte,
                            )
                        {
                            (
                                _,
                                _,
                                variable_info,
                                unresolved_method_refs,
                                unresolved_oref_method_refs,
                            ) = method.rebuild_method(
                                method_definition_node,
                                content,
                                &scope_tree,
                                method_type,
                                method_range,
                                public_variables_declared,
                                new_class_is_final,
                                old_is_final,
                                new_class_is_procedure_block,
                                &new_class_name,
                            );
                        }
                    }
                    MethodType::Procedure(_) => {
                        if let Some(method_definition_node) =
                            tree.root_node().named_descendant_for_byte_range(
                                method_range.start_byte,
                                method_range.end_byte,
                            )
                        {
                            (
                                _,
                                _,
                                variable_info,
                                unresolved_method_refs,
                                unresolved_oref_method_refs,
                            ) = method.rebuild_method(
                                method_definition_node,
                                content,
                                &scope_tree,
                                method_type,
                                method_range,
                                public_variables_declared,
                                new_class_is_final,
                                old_is_final,
                                new_class_is_procedure_block,
                                &new_class_name,
                            );
                        }
                    }
                    MethodType::Subroutine(_) | MethodType::Routine => {
                        (
                            _,
                            _,
                            variable_info,
                            unresolved_method_refs,
                            unresolved_oref_method_refs,
                        ) = method.rebuild_method(
                            tree.root_node(),
                            content,
                            &scope_tree,
                            method_type,
                            method_range,
                            public_variables_declared,
                            new_class_is_final,
                            old_is_final,
                            new_class_is_procedure_block,
                            &new_class_name,
                        );
                    }
                }

                for (variable, variable_range, variable_dependencies, variable_scope_id) in
                    variable_info
                {
                    let variable_name = variable.name.clone();
                    let variable_is_public = variable.is_public;
                    // add it to global semantic model (if public) or local semantic model/scope tree (if private)
                    // global semantic will add it to local semantic if private
                    let variable_ref = self.global_semantic_model.new_variable(
                        variable,
                        method_ref,
                        variable_scope_id,
                        variable_dependencies.clone(),
                        variable_range,
                        url.clone(),
                    );

                    // add variable ref and corresponding scope id to method
                    method
                        .variables
                        .entry(variable_name.clone())
                        .or_insert(Vec::new())
                        .push((variable_ref, variable_scope_id));

                    if variable_is_public {
                        scope_tree.new_public_var_symbol(
                            variable_name.clone(),
                            variable_range,
                            variable_ref,
                        );
                        self.pub_var_defs
                            .entry(variable_name)
                            .or_insert(HashMap::new())
                            .entry(method_ref)
                            .or_insert(HashMap::new())
                            .entry(variable_scope_id)
                            .or_insert(Vec::new())
                            .push(variable_ref);
                    } else {
                        scope_tree.new_variable_symbol(
                            variable_name,
                            variable_range,
                            variable_dependencies,
                            variable_ref,
                        );
                    }
                }
                self.dependency_graph.get_or_add_node(method_ref);
                self.method_defs
                    .entry(new_class_name.clone())
                    .or_insert_with(HashMap::new)
                    .insert(method_name.clone(), method_ref);
                self.resolve_method_references(&unresolved_method_refs, method_ref, old_class_id);
                unresolved_orefs.insert(method_ref, unresolved_oref_method_refs);
                if !method.is_public {
                    // creates method symbol in scope tree (private)
                    scope_tree.new_method_symbol(
                        method_name.clone(),
                        method_range,
                        method_ref,
                        url.clone(),
                    );
                }
                // adds method to global semantic model if public, and to local semantic model if private
                // also, creates method symbol if public
                self.global_semantic_model.new_method(
                    method,
                    method_ref,
                    method_range,
                    url.clone(),
                );
                self.compute_inheritance_override_index_method(
                    &classes_to_fully_recompute_inheritance,
                    &subclasses_to_recompute_inheritance,
                    method_name,
                    method_ref,
                    true,
                    new_class_is_final.unwrap_or(false),
                );
            }
            for (method_ref, unresolved_oref_methods) in unresolved_orefs {
                for (oref_name, oref_method_name, method_call_range, current_method_name) in
                    unresolved_oref_methods
                {
                    let (resolved, unresolved) = self.resolve_oref_methods(
                        method_ref,
                        &oref_name,
                        &oref_method_name,
                        method_call_range,
                        &current_method_name,
                        &scope_tree,
                    );
                    for (key, value) in unresolved {
                        self.unresolved_method_references
                            .entry(key)
                            .or_insert(HashSet::new())
                            .extend(value);
                    }
                    for referenced_method_ref in &resolved {
                        self.dependency_graph.add_edge(
                            method_ref,
                            *referenced_method_ref,
                            method_call_range,
                        );
                    }
                }
            }

            for (property_name, (property, property_range, property_ref)) in
                properties_already_rebuilt
            {
                if !property.is_public {
                    // creates and stores property symbol in scope tree
                    scope_tree.new_property_symbol(
                        property_name.clone(),
                        property_range,
                        property_ref,
                        url.clone(),
                    )
                }
                // adds property to gsm if public, lsm if private
                // if public, creates and stores property symbol in gsm
                self.global_semantic_model.new_property(
                    property,
                    property_ref,
                    property_range,
                    url.clone(),
                );
                self.property_defs
                    .entry(new_class_name.clone())
                    .or_insert_with(HashMap::new)
                    .insert(property_name.clone(), property_ref);

                self.compute_inheritance_override_index_property(
                    &classes_to_fully_recompute_inheritance,
                    &subclasses_to_recompute_inheritance,
                    property_name,
                    property_ref,
                    true,
                    new_class_is_final.unwrap_or(false),
                );
            }

            for (parameter_name, (parameter, parameter_range, parameter_ref)) in
                parameters_already_rebuilt
            {
                // creates property symbol and stores it in global semantic model
                // also stores parameter in gsm
                self.global_semantic_model.new_parameter(
                    parameter,
                    parameter_ref,
                    parameter_range,
                    url.clone(),
                );
                // add property ref to workspace
                self.parameter_defs
                    .entry(new_class_name.clone())
                    .or_insert_with(HashMap::new)
                    .insert(parameter_name.clone(), parameter_ref);
                self.compute_inheritance_override_index_parameter(
                    &classes_to_fully_recompute_inheritance,
                    &subclasses_to_recompute_inheritance,
                    parameter_name,
                    parameter_ref,
                    true,
                    new_class_is_final.unwrap_or(false),
                );
            }

            let scope_tree_snapshot = scope_tree.clone();
            let mut curr_class_hash = HashSet::new();
            curr_class_hash.insert(old_class_id);
            if !new_class_is_final.unwrap_or(false) {
                if let Some(dependents) = self
                    .dependent_class_index
                    .direct_subclasses
                    .get(&old_class_id)
                {
                    curr_class_hash.extend(dependents)
                }
            }
            let mut unresolved_orefs = HashMap::new();
            for ts_range in changed_ranges {
                // this only gives direct children of the root scope (so methods, properties, etc)
                let scopes_within_changed_range = scope_tree_snapshot
                    .find_scopes_in_range(ts_range.start_point, ts_range.end_point);
                // this can cover multiple methods
                for (_, curr_scope) in scopes_within_changed_range {
                    if let Some(method_name) = &curr_scope.method {
                        if methods_already_rebuilt.contains(method_name) {
                            continue;
                        }
                        methods_already_rebuilt.insert(method_name.clone());

                        let Some((method_range, method_type, public_variables_declared)) =
                            all_methods.get(method_name)
                        else {
                            continue;
                        };
                        let mut unresolved_method_refs = HashSet::new();
                        let mut unresolved_oref_method_refs = HashSet::new();
                        let mut variable_info = Vec::new();
                        let mut method_is_final_changed = false;
                        let mut method_is_public_changed = false;
                        let Some(method_ref) = self
                            .method_defs
                            .get(&new_class_name)
                            .and_then(|methods| methods.get(method_name))
                            .copied()
                        else {
                            eprintln!("error: method DNE");
                            continue;
                        };
                        if let Some(method) = self.global_semantic_model.get_mut_method(&method_ref)
                        {
                            match method_type {
                                MethodType::ClassMethod | MethodType::InstanceMethod => {
                                    if let Some(method_definition_node) =
                                        tree.root_node().named_descendant_for_byte_range(
                                            method_range.start_byte,
                                            method_range.end_byte,
                                        )
                                    {
                                        (
                                            method_is_final_changed,
                                            method_is_public_changed,
                                            variable_info,
                                            unresolved_method_refs,
                                            unresolved_oref_method_refs,
                                        ) = method.rebuild_method(
                                            method_definition_node,
                                            content,
                                            &scope_tree,
                                            *method_type,
                                            *method_range,
                                            public_variables_declared.clone(),
                                            new_class_is_final,
                                            old_is_final,
                                            new_class_is_procedure_block,
                                            &new_class_name,
                                        );
                                    }
                                }
                                MethodType::Procedure(_) => {
                                    if let Some(method_definition_node) =
                                        tree.root_node().named_descendant_for_byte_range(
                                            method_range.start_byte,
                                            method_range.end_byte,
                                        )
                                    {
                                        (
                                            method_is_final_changed,
                                            method_is_public_changed,
                                            variable_info,
                                            unresolved_method_refs,
                                            unresolved_oref_method_refs,
                                        ) = method.rebuild_method(
                                            method_definition_node,
                                            content,
                                            &scope_tree,
                                            *method_type,
                                            *method_range,
                                            public_variables_declared.clone(),
                                            new_class_is_final,
                                            old_is_final,
                                            new_class_is_procedure_block,
                                            &new_class_name,
                                        );
                                    }
                                }
                                MethodType::Subroutine(_) | MethodType::Routine => {
                                    (
                                        method_is_final_changed,
                                        method_is_public_changed,
                                        variable_info,
                                        unresolved_method_refs,
                                        unresolved_oref_method_refs,
                                    ) = method.rebuild_method(
                                        tree.root_node(),
                                        content,
                                        &scope_tree,
                                        *method_type,
                                        *method_range,
                                        public_variables_declared.clone(),
                                        new_class_is_final,
                                        old_is_final,
                                        new_class_is_procedure_block,
                                        &new_class_name,
                                    );
                                }
                            }
                        }

                        if method_is_final_changed {
                            self.compute_inheritance_override_index_method(
                                &classes_to_fully_recompute_inheritance,
                                &HashSet::new(),
                                method_name.clone(),
                                method_ref,
                                true,
                                new_class_is_final.unwrap_or(false),
                            );
                        }
                        if method_is_public_changed {
                            let mut method_is_public = true;
                            if let Some(method) = self.global_semantic_model.get_method(&method_ref)
                            {
                                if !method.is_public {
                                    method_is_public = false;
                                }
                            }
                            let node_index = self.dependency_graph.get_or_add_node(method_ref);
                            if !method_is_public {
                                let method_caller_refs = self
                                    .dependency_graph
                                    .remove_direct_ancestors(node_index, &curr_class_hash);
                                self.unresolved_method_references
                                    .entry((new_class_name.to_string(), method_name.clone()))
                                    .or_insert(HashSet::new())
                                    .extend(method_caller_refs);
                            }
                            self.global_semantic_model.change_method_publicity(
                                &method_ref,
                                *method_range,
                                url.clone(),
                            );
                        }
                        self.resolve_method_references(
                            &unresolved_method_refs,
                            method_ref,
                            old_class_id,
                        );
                        self.compute_inheritance_override_index_method(
                            &curr_class_hash,
                            &HashSet::new(),
                            method_name.clone(),
                            method_ref,
                            false,
                            new_class_is_final.unwrap_or(false),
                        );
                        unresolved_orefs.insert(method_ref, unresolved_oref_method_refs);
                        for (variable, variable_range, variable_dependencies, variable_scope_id) in
                            variable_info
                        {
                            let variable_name = variable.name.clone();
                            let variable_is_public = variable.is_public;
                            // add it to global semantic model (if public) or local semantic model/scope tree (if private)
                            // global semantic will add it to local semantic if private
                            let variable_ref = self.global_semantic_model.new_variable(
                                variable,
                                method_ref,
                                variable_scope_id,
                                variable_dependencies.clone(),
                                variable_range,
                                url.clone(),
                            );

                            if let Some(method) =
                                self.global_semantic_model.get_mut_method(&method_ref)
                            {
                                // add variable ref and corresponding scope id to method
                                method
                                    .variables
                                    .entry(variable_name.clone())
                                    .or_insert(Vec::new())
                                    .push((variable_ref, variable_scope_id));
                            }

                            if variable_is_public {
                                scope_tree.new_public_var_symbol(
                                    variable_name.clone(),
                                    variable_range,
                                    variable_ref,
                                );
                                self.pub_var_defs
                                    .entry(variable_name)
                                    .or_insert(HashMap::new())
                                    .entry(method_ref)
                                    .or_insert(HashMap::new())
                                    .entry(variable_scope_id)
                                    .or_insert(Vec::new())
                                    .push(variable_ref);
                            } else {
                                scope_tree.new_variable_symbol(
                                    variable_name,
                                    variable_range,
                                    variable_dependencies,
                                    variable_ref,
                                );
                            }
                        }
                    } else {
                        continue;
                    }
                }
            }

            for (method_ref, unresolved_oref_methods) in unresolved_orefs {
                for (oref_name, oref_method_name, method_call_range, current_method_name) in
                    unresolved_oref_methods
                {
                    let (resolved, unresolved) = self.resolve_oref_methods(
                        method_ref,
                        &oref_name,
                        &oref_method_name,
                        method_call_range,
                        &current_method_name,
                        &scope_tree,
                    );
                    for (key, value) in unresolved {
                        self.unresolved_method_references
                            .entry(key)
                            .or_insert(HashSet::new())
                            .extend(value);
                    }
                    for referenced_method_ref in &resolved {
                        self.dependency_graph.add_edge(
                            method_ref,
                            *referenced_method_ref,
                            method_call_range,
                        );
                    }
                }
            }
            let Some(doc) = self.get_document_mut(&url) else {
                return false;
            };
            doc.scope_tree = scope_tree;
            for (method_name, _) in all_methods {
                if methods_already_rebuilt.contains(&method_name) {
                    continue;
                }
                if !classes_to_fully_recompute_inheritance.is_empty() {
                    let extended_methods = self.build_override_index_for_method(
                        &classes_to_fully_recompute_inheritance,
                        &method_name,
                    );
                    for (extended_class_name, method_ref_map) in extended_methods {
                        self.method_defs
                            .entry(extended_class_name.clone())
                            .or_insert(HashMap::new())
                            .extend(method_ref_map);
                    }
                }
            }

            return true;
        }
        return false;
    }

    /// Resolves oref method references for a given method. Searches local scopes, parent scopes,
    /// child scopes, and then walks the dependency graph to find where the oref variable is defined.
    /// Returns the set of MethodRefs that the oref could be calling.
    pub fn resolve_oref_methods(
        &self,
        method_ref: MethodRef,
        oref_name: &str,
        oref_method_name: &str,
        method_call_range: Range,
        current_method_name: &str,
        scope_tree: &ScopeTree,
    ) -> (
        HashSet<MethodRef>,
        HashMap<(String, String), HashSet<(MethodRef, Range)>>,
    ) {
        let mut all_possible_oref_methods = HashSet::new();
        let mut unresolved_method_references = HashMap::new();

        let Some((scope_id, scope)) = scope_tree.get_scope(method_call_range.start_point) else {
            return (all_possible_oref_methods, unresolved_method_references);
        };

        let oref_is_public =
            if let Some(method) = self.global_semantic_model.get_method(&method_ref) {
                method
                    .public_variables_declared
                    .contains(&oref_name.to_string())
                    || method.method_type == MethodType::Routine
                    || matches!(method.method_type, MethodType::Subroutine(_))
            } else {
                false
            };

        if let Some((oref_def_range, oref_class_name)) =
            self.global_semantic_model.get_oref_in_scope_before_range(
                method_ref,
                scope_id,
                oref_name,
                method_call_range,
                &scope.variable_symbols,
            )
        {
            if let Some(referenced_method_ref) = self
                .method_defs
                .get(&oref_class_name)
                .and_then(|methods| methods.get(oref_method_name))
            {
                all_possible_oref_methods.insert(*referenced_method_ref);
            } else {
                unresolved_method_references
                    .entry((oref_class_name, oref_method_name.to_string()))
                    .or_insert(HashSet::new())
                    .insert((method_ref, oref_def_range));
            }
        } else if let Some(method_scope_id) =
            scope_tree.find_scope_by_method_name(current_method_name)
            && let Some(method_scope) = scope_tree.scopes.get(&method_scope_id)
        {
            let potential_scopes: Vec<ScopeId>;
            if method_scope_id != scope_id {
                if let Some((oref_def_range, oref_class_name)) =
                    self.global_semantic_model.get_oref_in_scope_before_range(
                        method_ref,
                        method_scope_id,
                        oref_name,
                        method_call_range,
                        &scope.variable_symbols,
                    )
                {
                    if let Some(referenced_method_ref) = self
                        .method_defs
                        .get(&oref_class_name)
                        .and_then(|methods| methods.get(oref_method_name))
                    {
                        all_possible_oref_methods.insert(*referenced_method_ref);
                        potential_scopes = scope_tree.get_children_before_scope_id(
                            method_call_range.start_point,
                            Some(oref_def_range.start_point),
                            &method_scope.children,
                        );
                    } else {
                        unresolved_method_references
                            .entry((oref_class_name, oref_method_name.to_string()))
                            .or_insert(HashSet::new())
                            .insert((method_ref, oref_def_range));
                        potential_scopes = scope_tree.get_children_before_scope_id(
                            method_call_range.start_point,
                            None,
                            &method_scope.children,
                        );
                    }
                } else {
                    potential_scopes = scope_tree.get_children_before_scope_id(
                        method_call_range.start_point,
                        None,
                        &method_scope.children,
                    );
                }
            } else {
                potential_scopes = scope_tree.get_children_before_scope_id(
                    method_call_range.start_point,
                    None,
                    &method_scope.children,
                );
            }
            for child_scope_id in potential_scopes {
                if let Some((oref_def_range, oref_class_name)) =
                    self.global_semantic_model.get_oref_in_scope_before_range(
                        method_ref,
                        child_scope_id,
                        oref_name,
                        method_call_range,
                        &scope.variable_symbols,
                    )
                {
                    if let Some(referenced_method_ref) = self
                        .method_defs
                        .get(&oref_class_name)
                        .and_then(|methods| methods.get(oref_method_name))
                    {
                        all_possible_oref_methods.insert(*referenced_method_ref);
                    } else {
                        unresolved_method_references
                            .entry((oref_class_name, oref_method_name.to_string()))
                            .or_insert(HashSet::new())
                            .insert((method_ref, oref_def_range));
                    }
                }
            }
            if all_possible_oref_methods.is_empty() && oref_is_public {
                let Some(&node_index) = self.dependency_graph.get_node(method_ref) else {
                    return (all_possible_oref_methods, unresolved_method_references);
                };

                let mut visited = HashSet::new();
                let mut queue = std::collections::VecDeque::new();
                visited.insert(node_index);

                for edge in self
                    .dependency_graph
                    .graph
                    .edges_directed(node_index, petgraph::Direction::Incoming)
                {
                    let parent = edge.source();
                    if visited.insert(parent) {
                        queue.push_back((parent, *edge.weight()));
                    }
                }

                while let Some((node, call_range)) = queue.pop_front() {
                    let ancestor_ref = self.dependency_graph.graph[node];
                    let mut found_def = false;

                    if let Some(scopes) = self
                        .pub_var_defs
                        .get(oref_name)
                        .and_then(|m| m.get(&ancestor_ref))
                    {
                        for (sid, variable_refs) in scopes {
                            for variable_ref in variable_refs {
                                if let Some(var_id) = variable_ref.pub_id
                                    && let Some(symbol) = self
                                        .global_semantic_model
                                        .get_variable_symbol(&ancestor_ref, var_id.0, sid)
                                    && symbol.location.end_byte < call_range.start_byte
                                {
                                    found_def = true;
                                    if let Some(var) = self
                                        .global_semantic_model
                                        .variables
                                        .get(&ancestor_ref)
                                        .and_then(|s| s.get(sid))
                                        .and_then(|vars| vars.get(var_id.0))
                                        && var.is_oref
                                        && let Some(ref oref_cls) = var.cls
                                    {
                                        if let Some(referenced_method_ref) = self
                                            .method_defs
                                            .get(oref_cls)
                                            .and_then(|methods| methods.get(oref_method_name))
                                        {
                                            all_possible_oref_methods
                                                .insert(*referenced_method_ref);
                                        } else {
                                            unresolved_method_references
                                                .entry((
                                                    oref_cls.to_string(),
                                                    oref_method_name.to_string(),
                                                ))
                                                .or_insert(HashSet::new())
                                                .insert((method_ref, symbol.location));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !found_def {
                        for edge in self
                            .dependency_graph
                            .graph
                            .edges_directed(node, petgraph::Direction::Incoming)
                        {
                            let parent = edge.source();
                            if visited.insert(parent) {
                                queue.push_back((parent, *edge.weight()));
                            }
                        }
                    }
                }
            }
        }

        return (all_possible_oref_methods, unresolved_method_references);
    }

    /// Rebuild keyword inheritance (is_procedure_block, default_language, is_final)
    /// for a single class by walking up the primary parent chain.
    pub fn rebuild_keyword_inheritance_for_class(&mut self, class_id: &ClassId) {
        let Some(class) = self.global_semantic_model.get_class(class_id) else {
            return;
        };
        if class.is_procedure_block.is_some()
            && class.default_language.is_some()
            && class.is_final.is_some()
        {
            return;
        }

        let mut pb = class.is_procedure_block;
        let mut lang = class.default_language.clone();
        let mut is_final = class.is_final;
        let mut current_parents = class.inherited_classes.clone();
        let mut visited = HashSet::new();

        while pb.is_none() || lang.is_none() || is_final.is_none() {
            let Some(parent_name) = current_parents.get(0) else {
                break;
            };
            if !visited.insert(parent_name.clone()) {
                break;
            }
            let Some(&parent_id) = self.classes.get(parent_name) else {
                break;
            };
            let Some(parent) = self.global_semantic_model.get_class(&parent_id) else {
                break;
            };
            if pb.is_none() {
                pb = parent.is_procedure_block;
            }
            if lang.is_none() {
                lang = parent.default_language.clone();
            }
            if is_final.is_none() {
                is_final = parent.is_final;
            }
            current_parents = parent.inherited_classes.clone();
        }

        if let Some(class) = self.global_semantic_model.get_mut_class(class_id) {
            if class.is_procedure_block.is_none() {
                class.is_procedure_block = pb;
            }
            if class.default_language.is_none() {
                class.default_language = lang;
            }
            if class.is_final.is_none() {
                class.is_final = is_final;
            }
        }
    }

    /// Rebuild the override index for a specific set of classes.
    ///
    /// Clears all override entries belonging to the affected classes, then recomputes
    /// their effective tables and override relationships using the (already correct)
    /// parent tables in the existing index.
    ///
    /// `affected_classes` should include the changed class AND all its transitive
    /// dependents (subclasses). Parent classes must NOT be in this set unless they
    /// also changed.
    pub fn build_override_index_for_classes(
        &mut self,
        affected_classes: &HashSet<ClassId>,
    ) -> (
        HashMap<String, HashMap<String, MethodRef>>,
        HashMap<String, HashMap<String, PropertyRef>>,
        HashMap<String, HashMap<String, ParameterRef>>,
    ) {
        let mut extended_methods = HashMap::new();
        let mut extended_properties = HashMap::new();
        let mut extended_parameters = HashMap::new();
        let mut cls_name_to_id = HashMap::new();
        let mut cls_id_to_name = HashMap::new();
        for class_id in affected_classes {
            if let Some(class_name) = self
                .global_semantic_model
                .get_class(class_id)
                .map(|c| c.name.clone())
            {
                cls_name_to_id.insert(class_name.clone(), *class_id);
                cls_id_to_name.insert(*class_id, class_name);
            }
        }
        // Clear old entries for affected classes
        for &class_id in affected_classes {
            let Some(class_name) = cls_id_to_name.get(&class_id) else {
                continue;
            };
            self.override_index.effective_methods.remove(class_name);
            self.override_index.effective_properties.remove(class_name);
            self.override_index.effective_parameters.remove(class_name);

            self.override_index
                .method_overrides
                .retain(|child, _| child.class != class_id);
            self.override_index
                .property_overrides
                .retain(|child, _| child.class != class_id);
            self.override_index
                .parameter_overrides
                .retain(|child, _| child.class != class_id);

            self.override_index
                .method_overridden_by
                .retain(|parent, _| parent.class != class_id);
            self.override_index
                .property_overridden_by
                .retain(|parent, _| parent.class != class_id);
            self.override_index
                .parameter_overridden_by
                .retain(|parent, _| parent.class != class_id);

            for children in self.override_index.method_overridden_by.values_mut() {
                children.retain(|child| child.class != class_id);
            }
            for children in self.override_index.property_overridden_by.values_mut() {
                children.retain(|child| child.class != class_id);
            }
            for children in self.override_index.parameter_overridden_by.values_mut() {
                children.retain(|child| child.class != class_id);
            }
        }

        // Rebuild in topological order via BFS from roots (classes whose parents
        // are all outside the affected set, i.e. already correct).
        let mut in_degree: HashMap<ClassId, usize> = affected_classes
            .iter()
            .map(|&class_id| {
                let dep_count = self
                    .global_semantic_model
                    .get_class(&class_id)
                    .map(|c| {
                        c.inherited_classes
                            .iter()
                            .filter(|&p| cls_name_to_id.contains_key(p))
                            .count()
                    })
                    .unwrap_or(0);
                (class_id, dep_count)
            })
            .collect();

        let mut queue: std::collections::VecDeque<ClassId> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(cid, _)| *cid)
            .collect();

        let mut ordered: Vec<ClassId> = Vec::with_capacity(affected_classes.len());
        while let Some(cid) = queue.pop_front() {
            ordered.push(cid);
            if let Some(dependents) = self.dependent_class_index.direct_subclasses.get(&cid) {
                for &child in dependents {
                    if let Some(deg) = in_degree.get_mut(&child) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(child);
                        }
                    }
                }
            }
        }
        for class_id in ordered {
            let Some(class) = self.global_semantic_model.get_class(&class_id) else {
                continue;
            };

            let class_name = class.name.clone();
            let inheritance_direction = class.inheritance_direction.clone();
            let parents = class.inherited_classes.clone();
            let methods: Vec<(String, MethodRef, bool)> = class
                .methods
                .iter()
                .map(|(name, mref)| {
                    let is_public = self.global_semantic_model.get_method(mref).is_some();
                    (name.clone(), *mref, is_public)
                })
                .collect();
            let properties: Vec<(String, PropertyRef, bool)> = class
                .properties
                .iter()
                .map(|(name, pref)| {
                    let is_public = self.global_semantic_model.get_property(pref).is_some();
                    (name.clone(), *pref, is_public)
                })
                .collect();
            let parameters: Vec<(String, ParameterRef)> = class
                .parameters
                .iter()
                .map(|(name, pref)| (name.clone(), *pref))
                .collect();

            // Build effective tables from parents
            let mut method_table: HashMap<String, (MethodRef, bool)> = HashMap::new();
            let mut property_table: HashMap<String, (PropertyRef, bool)> = HashMap::new();
            let mut parameter_table: HashMap<String, (ParameterRef, bool)> = HashMap::new();

            let parent_names: Vec<String> = if let Some(inheritance_direction) =
                inheritance_direction
                && inheritance_direction == "right"
            {
                parents.iter().rev().cloned().collect()
            } else {
                parents.iter().cloned().collect()
            };

            for parent_name in &parent_names {
                if let Some(parent_methods) = self.override_index.effective_methods.get(parent_name)
                {
                    for (name, mref) in parent_methods {
                        method_table.entry(name.clone()).or_insert((*mref, true));
                    }
                }
                if let Some(parent_props) =
                    self.override_index.effective_properties.get(parent_name)
                {
                    for (name, pref) in parent_props {
                        property_table.entry(name.clone()).or_insert((*pref, true));
                    }
                }
                if let Some(parent_params) =
                    self.override_index.effective_parameters.get(parent_name)
                {
                    for (name, pref) in parent_params {
                        parameter_table.entry(name.clone()).or_insert((*pref, true));
                    }
                }
            }

            // Overlay this class's parameters
            for (name, child_ref) in &parameters {
                if let Some((base_ref, true)) = parameter_table.get(name).copied() {
                    self.override_index
                        .parameter_overrides
                        .insert(*child_ref, base_ref);
                    self.override_index
                        .parameter_overridden_by
                        .entry(base_ref)
                        .or_default()
                        .push(*child_ref);
                }
                parameter_table.insert(name.clone(), (*child_ref, true));
            }

            // Overlay this class's methods
            for (name, child_ref, is_public) in &methods {
                if *is_public {
                    if let Some((base_ref, true)) = method_table.get(name).copied() {
                        self.override_index
                            .method_overrides
                            .insert(*child_ref, base_ref);
                        self.override_index
                            .method_overridden_by
                            .entry(base_ref)
                            .or_default()
                            .push(*child_ref);
                    }
                    method_table.insert(name.clone(), (*child_ref, true));
                } else {
                    if let Some((base_ref, _)) = method_table.get(name).copied() {
                        self.override_index
                            .method_overrides
                            .insert(*child_ref, base_ref);
                        self.override_index
                            .method_overridden_by
                            .entry(base_ref)
                            .or_default()
                            .push(*child_ref);
                    }
                    method_table.insert(name.clone(), (*child_ref, false));
                }
            }

            // Overlay this class's properties
            for (name, child_ref, is_public) in &properties {
                if *is_public {
                    if let Some((base_ref, true)) = property_table.get(name).copied() {
                        self.override_index
                            .property_overrides
                            .insert(*child_ref, base_ref);
                        self.override_index
                            .property_overridden_by
                            .entry(base_ref)
                            .or_default()
                            .push(*child_ref);
                    }
                    property_table.insert(name.clone(), (*child_ref, true));
                } else {
                    if let Some((base_ref, _)) = property_table.get(name).copied() {
                        self.override_index
                            .property_overrides
                            .insert(*child_ref, base_ref);
                        self.override_index
                            .property_overridden_by
                            .entry(base_ref)
                            .or_default()
                            .push(*child_ref);
                    }
                    property_table.insert(name.clone(), (*child_ref, false));
                }
            }

            // Store effective tables (all methods/properties/parameters this class has access to)
            let mut effective_methods: HashMap<String, MethodRef> = HashMap::new();
            for (method_name, (method_ref, _)) in method_table {
                effective_methods.insert(method_name.clone(), method_ref);
                if method_ref.class != class_id {
                    extended_methods
                        .entry(class_name.clone())
                        .or_insert(HashMap::new())
                        .insert(method_name, method_ref);
                }
            }
            self.override_index
                .effective_methods
                .insert(class_name.clone(), effective_methods);

            let mut effective_properties: HashMap<String, PropertyRef> = HashMap::new();
            for (property_name, (property_ref, _)) in property_table {
                effective_properties.insert(property_name.clone(), property_ref);
                if property_ref.class != class_id {
                    extended_properties
                        .entry(class_name.clone())
                        .or_insert(HashMap::new())
                        .insert(property_name, property_ref);
                }
            }
            self.override_index
                .effective_properties
                .insert(class_name.clone(), effective_properties);

            let mut effective_parameters: HashMap<String, ParameterRef> = HashMap::new();
            for (parameter_name, (parameter_ref, _)) in parameter_table {
                effective_parameters.insert(parameter_name.clone(), parameter_ref);
                if parameter_ref.class != class_id {
                    extended_parameters
                        .entry(class_name.clone())
                        .or_insert(HashMap::new())
                        .insert(parameter_name, parameter_ref);
                }
            }
            self.override_index
                .effective_parameters
                .insert(class_name.clone(), effective_parameters);
        }
        (extended_methods, extended_properties, extended_parameters)
    }

    pub fn build_override_index_for_property(
        &mut self,
        affected_classes: &HashSet<ClassId>,
        property_name: &str,
    ) -> HashMap<String, HashMap<String, PropertyRef>> {
        let mut extended_properties = HashMap::new();
        let mut cls_name_to_id = HashMap::new();
        let mut cls_id_to_name = HashMap::new();
        for class_id in affected_classes {
            if let Some(class_name) = self
                .global_semantic_model
                .get_class(class_id)
                .map(|c| c.name.clone())
            {
                cls_name_to_id.insert(class_name.clone(), *class_id);
                cls_id_to_name.insert(*class_id, class_name);
            }
        }
        // 1. Clear old entries for this method name in affected classes
        for &class_id in affected_classes {
            let Some(class_name) = cls_id_to_name.get(&class_id) else {
                continue;
            };
            // Remove from effective tables
            if let Some(properties) = self.override_index.effective_properties.get_mut(class_name) {
                properties.remove(property_name);
            }

            // Remove property_overrides where child belongs to affected class and has this name
            let overrides_to_remove: Vec<PropertyRef> = self
                .override_index
                .property_overrides
                .keys()
                .filter(|child_ref| {
                    child_ref.class == class_id
                        && self
                            .global_semantic_model
                            .get_class(&class_id)
                            .and_then(|c| {
                                c.properties
                                    .iter()
                                    .find(|(_, pref)| **pref == **child_ref)
                                    .map(|(name, _)| name == property_name)
                            })
                            .unwrap_or(false)
                })
                .copied()
                .collect();
            for child_ref in &overrides_to_remove {
                if let Some(parent_ref) = self.override_index.property_overrides.remove(child_ref) {
                    if let Some(children) = self
                        .override_index
                        .property_overridden_by
                        .get_mut(&parent_ref)
                    {
                        children.retain(|c| c != child_ref);
                    }
                }
            }

            // Remove property_overridden_by where parent belongs to affected class with this name
            let overridden_by_to_remove: Vec<PropertyRef> = self
                .override_index
                .property_overridden_by
                .keys()
                .filter(|parent_ref| {
                    parent_ref.class == class_id
                        && self
                            .global_semantic_model
                            .get_class(&class_id)
                            .and_then(|c| {
                                c.properties
                                    .iter()
                                    .find(|(_, pref)| **pref == **parent_ref)
                                    .map(|(name, _)| name == property_name)
                            })
                            .unwrap_or(false)
                })
                .copied()
                .collect();
            for parent_ref in &overridden_by_to_remove {
                if let Some(children) = self
                    .override_index
                    .property_overridden_by
                    .remove(parent_ref)
                {
                    for child_ref in children {
                        self.override_index.property_overrides.remove(&child_ref);
                    }
                }
            }
        }

        // 2. Topological sort (same Kahn's algorithm as full rebuild)
        let mut in_degree: HashMap<ClassId, usize> = affected_classes
            .iter()
            .map(|&class_id| {
                let dep_count = self
                    .global_semantic_model
                    .get_class(&class_id)
                    .map(|c| {
                        c.inherited_classes
                            .iter()
                            .filter(|&p| cls_name_to_id.contains_key(p))
                            .count()
                    })
                    .unwrap_or(0);
                (class_id, dep_count)
            })
            .collect();

        let mut queue: std::collections::VecDeque<ClassId> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(cid, _)| *cid)
            .collect();

        let mut ordered: Vec<ClassId> = Vec::with_capacity(affected_classes.len());
        while let Some(cid) = queue.pop_front() {
            ordered.push(cid);
            if let Some(dependents) = self.dependent_class_index.direct_subclasses.get(&cid) {
                for &child in dependents {
                    if let Some(deg) = in_degree.get_mut(&child) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(child);
                        }
                    }
                }
            }
        }
        // 3. Rebuild just this method's entry for each affected class in order
        for class_id in ordered {
            let Some(class) = self.global_semantic_model.get_class(&class_id) else {
                continue;
            };

            let class_name = class.name.clone();
            let inheritance_direction = class.inheritance_direction.clone();
            let parents = class.inherited_classes.clone();
            let own_property: Option<(PropertyRef, bool)> =
                class.properties.get(property_name).map(|pref| {
                    let is_public = self.global_semantic_model.get_property(pref).is_some();
                    (*pref, is_public)
                });

            // Find inherited entry for this method name from parents
            let mut inherited_entry: Option<(PropertyRef, bool)> = None;

            let parent_names: Vec<String> = if let Some(inheritance_direction) =
                inheritance_direction
                && inheritance_direction == "right"
            {
                parents.iter().rev().cloned().collect()
            } else {
                parents.iter().cloned().collect()
            };

            for parent_name in &parent_names {
                if let Some(parent_properties) =
                    self.override_index.effective_properties.get(parent_name)
                {
                    if let Some(&parent_pref) = parent_properties.get(property_name) {
                        inherited_entry = Some((parent_pref, true));
                        break; // first-wins semantics
                    }
                }
            }

            // Determine the effective entry and record overrides
            let effective: Option<(PropertyRef, bool)> = match (own_property, inherited_entry) {
                (Some((child_ref, is_public)), Some((base_ref, _))) => {
                    // Child overrides parent
                    self.override_index
                        .property_overrides
                        .insert(child_ref, base_ref);
                    self.override_index
                        .property_overridden_by
                        .entry(base_ref)
                        .or_default()
                        .push(child_ref);
                    Some((child_ref, is_public))
                }
                (Some((child_ref, is_public)), None) => Some((child_ref, is_public)),
                (None, Some(inherited)) => Some(inherited),
                (None, None) => None,
            };

            // Update effective table
            if let Some((pref, _)) = effective {
                self.override_index
                    .effective_properties
                    .entry(class_name.clone())
                    .or_default()
                    .insert(property_name.to_string(), pref);
                if pref.class != class_id {
                    extended_properties
                        .entry(class_name)
                        .or_insert(HashMap::new())
                        .insert(property_name.to_string(), pref);
                }
            }
        }
        return extended_properties;
    }

    pub fn build_override_index_for_parameter(
        &mut self,
        affected_classes: &HashSet<ClassId>,
        parameter_name: &str,
    ) -> HashMap<String, HashMap<String, ParameterRef>> {
        let mut extended_parameters = HashMap::new();
        let mut cls_name_to_id = HashMap::new();
        let mut cls_id_to_name = HashMap::new();
        for class_id in affected_classes {
            if let Some(class_name) = self
                .global_semantic_model
                .get_class(class_id)
                .map(|c| c.name.clone())
            {
                cls_name_to_id.insert(class_name.clone(), *class_id);
                cls_id_to_name.insert(*class_id, class_name);
            }
        }
        // 1. Clear old entries for this parameter name in affected classes
        for &class_id in affected_classes {
            let Some(class_name) = cls_id_to_name.get(&class_id) else {
                continue;
            };
            // Remove from effective tables
            if let Some(parameters) = self.override_index.effective_parameters.get_mut(class_name) {
                parameters.remove(parameter_name);
            }

            // Remove parameter_overrides where child belongs to affected class and has this name
            let overrides_to_remove: Vec<ParameterRef> = self
                .override_index
                .parameter_overrides
                .keys()
                .filter(|child_ref| {
                    child_ref.class == class_id
                        && self
                            .global_semantic_model
                            .get_class(&class_id)
                            .and_then(|c| {
                                c.parameters
                                    .iter()
                                    .find(|(_, param_ref)| **param_ref == **child_ref)
                                    .map(|(name, _)| name == parameter_name)
                            })
                            .unwrap_or(false)
                })
                .copied()
                .collect();
            for child_ref in &overrides_to_remove {
                if let Some(parent_ref) = self.override_index.parameter_overrides.remove(child_ref)
                {
                    if let Some(children) = self
                        .override_index
                        .parameter_overridden_by
                        .get_mut(&parent_ref)
                    {
                        children.retain(|c| c != child_ref);
                    }
                }
            }

            // Remove parameter_overridden_by where parent belongs to affected class with this name
            let overridden_by_to_remove: Vec<ParameterRef> = self
                .override_index
                .parameter_overridden_by
                .keys()
                .filter(|parent_ref| {
                    parent_ref.class == class_id
                        && self
                            .global_semantic_model
                            .get_class(&class_id)
                            .and_then(|c| {
                                c.parameters
                                    .iter()
                                    .find(|(_, param_ref)| **param_ref == **parent_ref)
                                    .map(|(name, _)| name == parameter_name)
                            })
                            .unwrap_or(false)
                })
                .copied()
                .collect();
            for parent_ref in &overridden_by_to_remove {
                if let Some(children) = self
                    .override_index
                    .parameter_overridden_by
                    .remove(parent_ref)
                {
                    for child_ref in children {
                        self.override_index.parameter_overrides.remove(&child_ref);
                    }
                }
            }
        }

        // 2. Topological sort (same Kahn's algorithm as full rebuild)
        let mut in_degree: HashMap<ClassId, usize> = affected_classes
            .iter()
            .map(|&class_id| {
                let dep_count = self
                    .global_semantic_model
                    .get_class(&class_id)
                    .map(|c| {
                        c.inherited_classes
                            .iter()
                            .filter(|&p| cls_name_to_id.contains_key(p))
                            .count()
                    })
                    .unwrap_or(0);
                (class_id, dep_count)
            })
            .collect();

        let mut queue: std::collections::VecDeque<ClassId> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(cid, _)| *cid)
            .collect();

        let mut ordered: Vec<ClassId> = Vec::with_capacity(affected_classes.len());
        while let Some(cid) = queue.pop_front() {
            ordered.push(cid);
            if let Some(dependents) = self.dependent_class_index.direct_subclasses.get(&cid) {
                for &child in dependents {
                    if let Some(deg) = in_degree.get_mut(&child) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(child);
                        }
                    }
                }
            }
        }

        // 3. Rebuild just this method's entry for each affected class in order
        for class_id in ordered {
            let Some(class) = self.global_semantic_model.get_class(&class_id) else {
                continue;
            };

            let class_name = class.name.clone();
            let inheritance_direction = class.inheritance_direction.clone();
            let parents = class.inherited_classes.clone();
            let own_parameter: Option<(ParameterRef, bool)> =
                class.parameters.get(parameter_name).map(|pref| {
                    let is_public = self.global_semantic_model.get_parameter(pref).is_some();
                    (*pref, is_public)
                });

            // Find inherited entry for this method name from parents
            let mut inherited_entry: Option<(ParameterRef, bool)> = None;

            let parent_names: Vec<String> = if let Some(inheritance_direction) =
                inheritance_direction
                && inheritance_direction == "right"
            {
                parents.iter().rev().cloned().collect()
            } else {
                parents.iter().cloned().collect()
            };

            for parent_name in &parent_names {
                if let Some(parent_parameters) =
                    self.override_index.effective_parameters.get(parent_name)
                {
                    if let Some(&parent_paramref) = parent_parameters.get(parameter_name) {
                        inherited_entry = Some((parent_paramref, true));
                        break; // first-wins semantics
                    }
                }
            }

            // Determine the effective entry and record overrides
            let effective: Option<(ParameterRef, bool)> = match (own_parameter, inherited_entry) {
                (Some((child_ref, is_public)), Some((base_ref, _))) => {
                    // Child overrides parent
                    self.override_index
                        .parameter_overrides
                        .insert(child_ref, base_ref);
                    self.override_index
                        .parameter_overridden_by
                        .entry(base_ref)
                        .or_default()
                        .push(child_ref);
                    Some((child_ref, is_public))
                }
                (Some((child_ref, is_public)), None) => Some((child_ref, is_public)),
                (None, Some(inherited)) => Some(inherited),
                (None, None) => None,
            };

            // Update effective table
            if let Some((paramref, _)) = effective {
                self.override_index
                    .effective_parameters
                    .entry(class_name.clone())
                    .or_default()
                    .insert(parameter_name.to_string(), paramref);
                if paramref.class != class_id {
                    extended_parameters
                        .entry(class_name)
                        .or_insert(HashMap::new())
                        .insert(parameter_name.to_string(), paramref);
                }
            }
        }
        return extended_parameters;
    }

    /// Rebuilds override index entries for a single method name across a set of affected classes.
    /// Only touches override entries where the method name matches — much cheaper than
    /// `build_override_index_for_classes` when a single method was added/removed/renamed.
    /// TODO: Does this rebuild everything over and over again if I include the classid dependents?
    pub fn build_override_index_for_method(
        &mut self,
        affected_classes: &HashSet<ClassId>,
        method_name: &str,
    ) -> HashMap<String, HashMap<String, MethodRef>> {
        let mut extended_methods = HashMap::new();
        let mut cls_name_to_id = HashMap::new();
        let mut cls_id_to_name = HashMap::new();
        for class_id in affected_classes {
            if let Some(class_name) = self
                .global_semantic_model
                .get_class(class_id)
                .map(|c| c.name.clone())
            {
                cls_name_to_id.insert(class_name.clone(), *class_id);
                cls_id_to_name.insert(*class_id, class_name);
            }
        }
        // 1. Clear old entries for this method name in affected classes
        for &class_id in affected_classes {
            let Some(class_name) = cls_id_to_name.get(&class_id) else {
                continue;
            };
            // Remove from effective tables
            if let Some(methods) = self.override_index.effective_methods.get_mut(class_name) {
                methods.remove(method_name);
            }

            // Remove method_overrides where child belongs to affected class and has this name
            let overrides_to_remove: Vec<MethodRef> = self
                .override_index
                .method_overrides
                .keys()
                .filter(|child_ref| {
                    child_ref.class == class_id
                        && self
                            .global_semantic_model
                            .get_class(&class_id)
                            .and_then(|c| {
                                c.methods
                                    .iter()
                                    .find(|(_, mref)| **mref == **child_ref)
                                    .map(|(name, _)| name == method_name)
                            })
                            .unwrap_or(false)
                })
                .copied()
                .collect();
            for child_ref in &overrides_to_remove {
                if let Some(parent_ref) = self.override_index.method_overrides.remove(child_ref) {
                    if let Some(children) = self
                        .override_index
                        .method_overridden_by
                        .get_mut(&parent_ref)
                    {
                        children.retain(|c| c != child_ref);
                    }
                }
            }

            // Remove method_overridden_by where parent belongs to affected class with this name
            let overridden_by_to_remove: Vec<MethodRef> = self
                .override_index
                .method_overridden_by
                .keys()
                .filter(|parent_ref| {
                    parent_ref.class == class_id
                        && self
                            .global_semantic_model
                            .get_class(&class_id)
                            .and_then(|c| {
                                c.methods
                                    .iter()
                                    .find(|(_, mref)| **mref == **parent_ref)
                                    .map(|(name, _)| name == method_name)
                            })
                            .unwrap_or(false)
                })
                .copied()
                .collect();
            for parent_ref in &overridden_by_to_remove {
                if let Some(children) = self.override_index.method_overridden_by.remove(parent_ref)
                {
                    for child_ref in children {
                        self.override_index.method_overrides.remove(&child_ref);
                    }
                }
            }
        }

        // 2. Topological sort (same Kahn's algorithm as full rebuild)
        let mut in_degree: HashMap<ClassId, usize> = affected_classes
            .iter()
            .map(|&class_id| {
                let dep_count = self
                    .global_semantic_model
                    .get_class(&class_id)
                    .map(|c| {
                        c.inherited_classes
                            .iter()
                            .filter(|&p| cls_name_to_id.contains_key(p))
                            .count()
                    })
                    .unwrap_or(0);
                (class_id, dep_count)
            })
            .collect();

        let mut queue: std::collections::VecDeque<ClassId> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(cid, _)| *cid)
            .collect();
        let mut ordered: Vec<ClassId> = Vec::with_capacity(affected_classes.len());
        while let Some(cid) = queue.pop_front() {
            ordered.push(cid);
            if let Some(dependents) = self.dependent_class_index.direct_subclasses.get(&cid) {
                for &child in dependents {
                    if let Some(deg) = in_degree.get_mut(&child) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(child);
                        }
                    }
                }
            }
        }
        // 3. Rebuild just this method's entry for each affected class in order
        for class_id in ordered {
            let Some(class) = self.global_semantic_model.get_class(&class_id) else {
                continue;
            };

            let class_name = class.name.clone();
            let inheritance_direction = class.inheritance_direction.clone();
            let parents = class.inherited_classes.clone();
            let own_method: Option<(MethodRef, bool)> =
                class.methods.get(method_name).map(|mref| {
                    let is_public = self.global_semantic_model.get_method(mref).is_some();
                    (*mref, is_public)
                });
            // Find inherited entry for this method name from parents
            let mut inherited_entry: Option<(MethodRef, bool)> = None;

            let parent_names: Vec<String> = if let Some(inheritance_direction) =
                inheritance_direction
                && inheritance_direction == "right"
            {
                parents.iter().rev().cloned().collect()
            } else {
                parents.iter().cloned().collect()
            };

            for parent_name in &parent_names {
                if let Some(parent_methods) = self.override_index.effective_methods.get(parent_name)
                {
                    if let Some(&parent_mref) = parent_methods.get(method_name) {
                        inherited_entry = Some((parent_mref, true));
                        break; // first-wins semantics
                    }
                }
            }

            // Determine the effective entry and record overrides
            let effective: Option<(MethodRef, bool)> = match (own_method, inherited_entry) {
                (Some((child_ref, is_public)), Some((base_ref, _))) => {
                    // Child overrides parent
                    self.override_index
                        .method_overrides
                        .insert(child_ref, base_ref);
                    self.override_index
                        .method_overridden_by
                        .entry(base_ref)
                        .or_default()
                        .push(child_ref);
                    Some((child_ref, is_public))
                }
                (Some((child_ref, is_public)), None) => Some((child_ref, is_public)),
                (None, Some(inherited)) => Some(inherited),
                (None, None) => None,
            };

            // Update effective table
            if let Some((mref, _)) = effective {
                self.override_index
                    .effective_methods
                    .entry(class_name.clone())
                    .or_default()
                    .insert(method_name.to_string(), mref);
                if mref.class != class_id {
                    extended_methods
                        .entry(class_name)
                        .or_insert(HashMap::new())
                        .insert(method_name.to_string(), mref);
                }
            }
        }
        return extended_methods;
    }

    /// Fetch a tracked document by URL.
    ///
    /// Returns `None` and logs an error if the URL is not present in `self.documents`.
    fn get_document(&self, url: &Url) -> Option<&Document> {
        let Some(document) = self.documents.get(url) else {
            eprintln!("Error: Couldn't find document for url: {}", url.path());
            return None;
        };
        Some(document)
    }

    /// Fetch a tracked document by URL as a mutable reference.
    ///
    /// Returns `None` and logs an error if the URL is not present in `self.documents`.
    fn get_document_mut(&mut self, url: &Url) -> Option<&mut Document> {
        self.documents.get_mut(url)
    }

    /// Returns the `Url` and `Range` that point to the method location for `method_name`
    pub fn get_method_definition(
        &self,
        method_ref: &MethodRef,
        offset: Option<usize>,
    ) -> Vec<(Url, Range)> {
        let mut locations = Vec::new();
        if let Some(cls_sym) = self
            .global_semantic_model
            .get_class_symbol(&method_ref.class)
            && let Some(cls_doc) = self.get_document(&cls_sym.url)
        {
            if let Some(method_symbol) = self.global_semantic_model.get_method_symbol(method_ref) {
                locations.push((method_symbol.url.clone(), method_symbol.location));
                return locations;
            } else if let Some(method_symbol) =
                cls_doc.scope_tree.get_private_method_symbol(&method_ref)
            {
                let sym_range = if let Some(offset) = offset {
                    let content = &cls_doc.content;
                    let new_start_point = Point {
                        row: method_symbol.location.start_point.row + offset,
                        column: method_symbol.location.start_point.column,
                    };
                    let new_start_byte = point_to_byte(content, new_start_point);
                    let new_end_point = Point {
                        row: method_symbol.location.end_point.row + offset,
                        column: method_symbol.location.end_point.column,
                    };
                    let new_end_byte = point_to_byte(content, new_end_point);
                    let new_range = Range {
                        start_byte: new_start_byte,
                        start_point: new_start_point,
                        end_byte: new_end_byte,
                        end_point: new_end_point,
                    };
                    new_range
                } else {
                    method_symbol.location
                };
                locations.push((cls_sym.url.clone(), sym_range));
                return locations;
            }
        }

        return locations;
    }

    /// Finds all potential variable definitons for `variable_name`
    /// and finds the corresponding variables If there is another variable definition in the same scope
    /// and it comes after the first definition (but before either point or method_call based on the case),
    /// it will replace the first definition. If the definition is defined in the current class and method, then if the
    /// definition comes after `point` it is not added. Similarly, if the definition
    /// is from another class/method (and is connected by method calls, tracked in dependencyGraph),
    /// if the definition comes after the method call that connects it, it is not added.
    ///
    /// Returns a Vec of all potential locations (`Url`, `Range`) of the associated variable definition.
    pub fn get_variable_definition(
        &self,
        url: &Url,
        point: Point,
        variable_name: String,
    ) -> Vec<(Url, Range)> {
        let mut locations = Vec::new();
        let Some(document) = self.get_document(url) else {
            eprintln!(
                "Error: failed to get document for file {:?}. Aborting get_variable_definition",
                url.path()
            );
            return locations;
        };

        let class_name = document.class_name.clone();

        let Some(method_name) = document.scope_tree.get_method_name(point) else {
            return locations;
        };
        let Some(var_ref_scope_id) = document.scope_tree.find_current_scope(point) else {
            eprintln!(
                "Error: failed to find scope for variable range, returning (get_variable_definition)"
            );
            return locations;
        };

        if let Some(method_ref) = self
            .method_defs
            .get(&class_name)
            .and_then(|method_refs| method_refs.get(&method_name))
        {
            let private_var_ranges = document
                .scope_tree
                .get_variable_definition(variable_name.as_str(), var_ref_scope_id);
            if !private_var_ranges.is_empty() {
                let mut location_hash = HashMap::new();
                let mut seen_scope_ids = Vec::new();
                let mut scope_children = document.scope_tree.get_scope_children(&var_ref_scope_id);
                scope_children.insert(var_ref_scope_id);
                for (child_scope_id, variable_ranges) in private_var_ranges {
                    if !scope_children.contains(&child_scope_id) {
                        continue;
                    }
                    for var_range in variable_ranges {
                        if var_range.end_point < point {
                            if !seen_scope_ids.contains(&child_scope_id) {
                                let index = locations.len();
                                locations.push((url.clone(), var_range));
                                seen_scope_ids.push(child_scope_id);
                                location_hash.insert(child_scope_id, index);
                            } else if let Some(&index) = location_hash.get(&child_scope_id) {
                                let curr_indexed_sym_range = locations[index].1;
                                if curr_indexed_sym_range.end_byte < var_range.start_byte {
                                    locations[index] = (url.clone(), var_range);
                                }
                            }
                        }
                    }
                }
                if !locations.is_empty() {
                    return locations;
                }
            }

            let pub_var_refs = document
                .scope_tree
                .pub_variable_in_scope(variable_name.as_str(), var_ref_scope_id);
            let mut location_hash = HashMap::new();
            let mut seen_scope_ids = Vec::new();
            let mut scope_children = document.scope_tree.get_scope_children(&var_ref_scope_id);
            scope_children.insert(var_ref_scope_id);
            for (child_scope_id, variable_refs) in pub_var_refs {
                if !scope_children.contains(&child_scope_id) {
                    continue;
                }
                for variable_ref in variable_refs {
                    if let Some(var_id) = variable_ref.pub_id
                        && let Some(symbol) = self.global_semantic_model.get_variable_symbol(
                            method_ref,
                            var_id.0,
                            &child_scope_id,
                        )
                    {
                        if symbol.location.end_point < point {
                            if !seen_scope_ids.contains(&child_scope_id) {
                                let index = locations.len();
                                locations.push((symbol.url.clone(), symbol.location));
                                seen_scope_ids.push(child_scope_id);
                                location_hash.insert(child_scope_id, index);
                            } else if let Some(&index) = location_hash.get(&child_scope_id) {
                                let curr_indexed_sym_range = locations[index].1;
                                if curr_indexed_sym_range.end_byte < symbol.location.start_byte {
                                    locations[index] = (symbol.url.clone(), symbol.location);
                                }
                            }
                        }
                    }
                }
            }
            if !locations.is_empty() {
                return locations;
            }

            if self.is_variable_public(*method_ref, variable_name.clone()) {
                if let Some(&node_index) = self.dependency_graph.get_node(*method_ref)
                    && let Some(public_var_definitions) = self.pub_var_defs.get(&variable_name)
                {
                    let all_ancestors = self.dependency_graph.all_ancestors(node_index);
                    let mut found_depth: Option<usize> = None;
                    for (ancestor_ref, method_call_range, depth) in &all_ancestors {
                        if let Some(fd) = found_depth {
                            if *depth > fd {
                                break;
                            }
                        }
                        let Some(variable_refs_hash_map) = public_var_definitions.get(ancestor_ref)
                        else {
                            continue;
                        };
                        let Some(def_cls_sym) = self
                            .global_semantic_model
                            .get_class_symbol(&ancestor_ref.class)
                        else {
                            continue;
                        };
                        let Some(def_doc) = self.get_document(&def_cls_sym.url) else {
                            continue;
                        };
                        let Some(method_scope_id) = def_doc
                            .scope_tree
                            .find_current_scope(method_call_range.start_point)
                        else {
                            continue;
                        };
                        let mut location_hash = HashMap::new();
                        let mut seen_scope_ids = Vec::new();
                        let mut scope_children =
                            def_doc.scope_tree.get_scope_children(&method_scope_id);
                        scope_children.insert(method_scope_id);
                        for (child_scope_id, variable_refs) in variable_refs_hash_map {
                            if !scope_children.contains(child_scope_id) {
                                continue;
                            }
                            for variable_ref in variable_refs {
                                if let Some(var_id) = variable_ref.pub_id
                                    && let Some(symbol) = self
                                        .global_semantic_model
                                        .get_variable_symbol(ancestor_ref, var_id.0, child_scope_id)
                                {
                                    if symbol.location.end_byte < method_call_range.start_byte {
                                        if !seen_scope_ids.contains(child_scope_id) {
                                            let index = locations.len();
                                            locations.push((symbol.url.clone(), symbol.location));
                                            seen_scope_ids.push(*child_scope_id);
                                            location_hash.insert(child_scope_id, index);
                                        } else if let Some(&index) =
                                            location_hash.get(child_scope_id)
                                        {
                                            let curr_indexed_sym_range = locations[index].1;
                                            if curr_indexed_sym_range.end_byte
                                                < symbol.location.start_byte
                                            {
                                                locations[index] =
                                                    (symbol.url.clone(), symbol.location);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if !locations.is_empty() {
                            found_depth = Some(*depth);
                        }
                    }
                    if !locations.is_empty() {
                        return locations;
                    }
                }
            }
        }
        locations
    }

    /// Resolve an object-reference method call to its definition location(s).
    pub fn get_oref_definitions(
        &self,
        oref_name: &str,
        oref_method_name: &str,
        curr_class: &str,
        oref_ref_range: Range,
        resolve_method: bool,
    ) -> Vec<(Url, Range)> {
        let curr_class_url = if let Some(class_id) = self.classes.get(curr_class)
            && let Some(class_sym) = self.global_semantic_model.get_class_symbol(class_id)
        {
            class_sym.url.clone()
        } else {
            return Vec::new();
        };
        let Some(current_document) = self.get_document(&curr_class_url) else {
            return Vec::new();
        };
        let Some(curr_method_name) = current_document
            .scope_tree
            .get_method_name(oref_ref_range.start_point)
        else {
            return Vec::new();
        };
        let Some(current_method_ref) = self
            .method_defs
            .get(curr_class)
            .and_then(|method_refs| method_refs.get(&curr_method_name))
            .copied()
        else {
            return Vec::new();
        };
        let scope_tree = current_document.scope_tree.clone();

        let (resolved, _) = self.resolve_oref_methods(
            current_method_ref,
            oref_name,
            oref_method_name,
            oref_ref_range,
            &curr_method_name,
            &scope_tree,
        );

        if !resolve_method {
            // Return the locations of the oref variable definitions (not the method they point to)
            // For now, return class definition of the resolved oref classes
            let mut locations = Vec::new();
            for method_ref in &resolved {
                let class_id = method_ref.class;
                if let Some(class_sym) = self.global_semantic_model.get_class_symbol(&class_id) {
                    locations.push((class_sym.url.clone(), class_sym.location));
                }
            }
            locations
        } else {
            let mut oref_method_locations = Vec::new();
            for method_ref in &resolved {
                oref_method_locations.extend(self.get_method_definition(method_ref, None));
            }
            oref_method_locations
        }
    }

    /// Finds the method struct representing the method that `variable_name` is defined in
    /// and uses that to determine if the variable is public or private.
    ///
    /// Returns `true` if variable `variable_name` is public and `false` otherwise.
    pub fn is_variable_public(&self, method_ref: MethodRef, variable_name: String) -> bool {
        let method = if let Some(m) = self.global_semantic_model.get_method(&method_ref) {
            m
        } else if let Some(lsm) = self
            .global_semantic_model
            .get_local_semantic(&method_ref.class)
            && let Some(m) = lsm.get_method(&method_ref)
        {
            m
        } else {
            return false;
        };

        let cls_is_procedure_block =
            if let Some(c) = self.global_semantic_model.get_class(&method_ref.class) {
                c.is_procedure_block.unwrap_or(true)
            } else {
                return false;
            };

        if let Some(is_procedure_block) = method.is_procedure_block {
            if !is_procedure_block {
                return true;
            }
        } else if !cls_is_procedure_block {
            return true;
        } else if method.public_variables_declared.contains(&variable_name) {
            return true;
        }
        return false;
    }

    /// Returns the `Url` and `Range` associated with the class `class_name` location.
    pub fn get_class_definition(&self, class_name: &str) -> Vec<(Url, Range)> {
        let Some(class_sym_id) = self.classes.get(class_name) else {
            eprintln!(
                "Error: Failed to find a class_sym_id  in class: {:?}, aborting get_class_definition",
                class_name
            );
            return Vec::new();
        };
        let Some(sym) = self.global_semantic_model.get_class_symbol(class_sym_id) else {
            eprintln!(
                "Error: Failed to find a class symbol in global_semantic_model for class: {:?}, aborting get_class_definition",
                class_name
            );
            return Vec::new();
        };
        vec![(sym.url.clone(), sym.location)]
    }

    /// Returns locations of classes that override the given class of class_id
    pub fn get_class_implementations(&self, class_id: &ClassId) -> Vec<(Url, Range)> {
        let mut locations = Vec::new();
        let Some(dependent_class_ids) = self.dependent_class_index.dependent_classes.get(class_id)
        else {
            eprintln!("Error: Classid {:?} has no implementations", class_id);
            return locations;
        };

        for dep_id in dependent_class_ids {
            let Some(class) = self.global_semantic_model.get_class(dep_id) else {
                eprintln!(
                    "Error: Class of ClassId {:?} DNE, skipping (get_class_implementations)",
                    dep_id
                );
                continue;
            };

            let cls_name = &class.name;

            let overriding_subclass_class_symbol_id = match self.classes.get(cls_name).copied() {
                Some(id) => id,
                None => {
                    eprintln!(
                        "Error: Class Symbol ID of ClassId {:?} DNE, skipping (get_class_implementations)",
                        dep_id
                    );
                    continue;
                }
            };
            let Some(sym) = self
                .global_semantic_model
                .get_class_symbol(&overriding_subclass_class_symbol_id)
            else {
                eprintln!(
                    "Error: Class Symbol for symbol Id {:?} DNE, skipping (get_class_implementations)",
                    overriding_subclass_class_symbol_id
                );
                continue;
            };
            locations.push((sym.url.clone(), sym.location));
        }
        locations
    }

    /// Returns the location of the superclass method that the given subclass method method_overrides
    pub fn get_method_superclass(
        &self,
        method_name: String,
        class_id: &ClassId,
    ) -> Vec<(Url, Range)> {
        let method_name_str = method_name.as_str();
        let mut locations = Vec::new();
        let Some(class) = self.global_semantic_model.get_class(&class_id) else {
            eprintln!("Error: Class struct DNE, aborting (get_method_superclass)",);
            return locations;
        };

        if let Some(method_ref) = class.get_method_ref(method_name_str) {
            let superclass_method_ref = match self.override_index.method_overrides.get(&method_ref)
            {
                Some(v) => v,
                None => {
                    eprintln!(
                        "Error: Method {:?} in subclass {:?} doesn't override any superclass method",
                        method_name_str, class.name
                    );
                    return locations;
                }
            };

            if let Some(superclass_method_symbol) = self
                .global_semantic_model
                .get_method_symbol(superclass_method_ref)
            {
                locations.push((
                    superclass_method_symbol.url.clone(),
                    superclass_method_symbol.location,
                ));
            }
        }
        return locations;
    }

    /// Returns the location(s) of the superclass(es) that the subclass inherits
    pub fn get_class_superclasses(&self, class_id: &ClassId) -> Vec<(Url, Range)> {
        let mut locations = Vec::new();
        if let Some(class) = self.global_semantic_model.get_class(class_id) {
            for inherited_class_name in &class.inherited_classes {
                if let Some(inherited_class_id) = self.classes.get(inherited_class_name)
                    && let Some(inherited_class) =
                        self.global_semantic_model.get_class(inherited_class_id)
                {
                    let Some(inherited_class_sym) = self
                        .global_semantic_model
                        .get_class_symbol(inherited_class_id)
                    else {
                        eprintln!(
                            "Error: failed to get class symbol from global semantic model for class named  {:?}, skipping in get_class_superclasses",
                            &inherited_class.name
                        );
                        continue;
                    };
                    locations.push((
                        inherited_class_sym.url.clone(),
                        inherited_class_sym.location,
                    ))
                } else {
                    eprintln!("Error: Inherited Class struct DNE, skipping",);
                    continue;
                };
            }
        }
        locations
    }

    /// Return locations of methods that override a given public method.
    ///
    /// Looks up the current document's class, confirms `method_name` is a public method, then uses
    /// `override_index.method_overridden_by` to find overriding methods (public or private) in subclasses.
    ///
    /// Each returned `(Url, Range)` points to the overriding method's definition location.
    pub fn get_method_overrides(&self, method_ref: &MethodRef) -> Vec<(Url, Range)> {
        let mut locations = Vec::new();
        // ---- overridden-by list ----
        let method_overrides = match self.override_index.method_overridden_by.get(method_ref) {
            Some(v) => v,
            None => {
                return locations;
            }
        };
        for override_method_ref in method_overrides {
            let override_cls_url = if let Some(class_symbol) = self
                .global_semantic_model
                .get_class_symbol(&override_method_ref.class)
            {
                &class_symbol.url
            } else {
                continue;
            };

            if let Some(sym) = self
                .global_semantic_model
                .get_method_symbol(override_method_ref)
            {
                locations.push((sym.url.clone(), sym.location));
            } else if let Some(doc) = self.documents.get(override_cls_url)
                && let Some(sym) = doc
                    .scope_tree
                    .get_private_method_symbol(&override_method_ref)
            {
                locations.push((override_cls_url.clone(), sym.location));
            }
        }
        locations
    }
}

impl ProjectState {
    /// Create a new `ProjectState` with default configuration and empty indexing state.
    ///
    /// Initializes shared parsers, an empty `ProjectData` store, and leaves `project_root_path`
    /// unset (expected to be populated during LSP initialization).
    pub fn new() -> Self {
        Self {
            project_root_path: OnceLock::new(),
            parsers: WorkspaceParsers::new(),
            data: RwLock::new(ProjectData {
                config: Config::default(),
                documents: HashMap::new(),
                global_semantic_model: GlobalSemanticModel::new(),
                classes: HashMap::new(),
                method_defs: HashMap::new(),
                pub_var_defs: HashMap::new(),
                parameter_defs: HashMap::new(),
                property_defs: HashMap::new(),
                override_index: OverrideIndex::new(),
                dependent_class_index: Dependents::new(),
                dependency_graph: DependencyGraph::new(),
                unresolved_method_references: HashMap::new(),
                unresolved_inheritance_references: HashMap::new(),
            }),
        }
    }

    /// Handle an LSP `textDocument/didOpen` by parsing and committing the document.
    ///
    /// Parses the text with the appropriate Tree-sitter grammar, derives the class name for `.cls`
    /// files, then updates project state inside a single write lock:
    /// - Adds the document if new, or updates it if contents/type changed
    /// - Rebuilds inheritance/override/call/variable indexes for affected state
    pub fn handle_document_opened(
        &self,
        url: Url,
        text: String,
        file_type: FileType,
        version: i32,
    ) {
        // Parse OUTSIDE lock
        let tree = match file_type {
            FileType::Cls => match self.parsers.cls.lock().parse(&text, None) {
                Some(t) => t,
                None => {
                    eprintln!("parse failed for cls file with content: {}", text);
                    generic_exit_statements("ProjectState", "handle_document_opened");
                    return;
                }
            },
            FileType::Routine => match self.parsers.routine.lock().parse(&text, None) {
                Some(t) => t,
                None => {
                    eprintln!("parse failed for routine file with content: {}", text);
                    generic_exit_statements("ProjectState", "handle_document_opened");
                    return;
                }
            },
            FileType::Xml => match self.parsers.xml.lock().parse(&text, None) {
                Some(t) => t,
                None => {
                    eprintln!("parse failed for xml file with content: {}", text);
                    generic_exit_statements("ProjectState", "handle_document_opened");
                    return;
                }
            },
        };

        if file_type == FileType::Xml {
            let mut data = self.data.write();
            let existing_snapshot = data
                .documents
                .get(&url)
                .map(|d| (d.content.clone(), d.file_type.clone()));

            match existing_snapshot {
                None => {
                    data.add_document(
                        url,
                        text.as_str(),
                        &tree,
                        file_type,
                        None,
                        "XML".to_string(),
                        Some(version),
                        tree.root_node().range(),
                    );
                }
                Some((old_text, old_type)) => {
                    if old_text != text || old_type != file_type {
                        data.incremental_update_document(
                            url,
                            &tree,
                            file_type,
                            version,
                            &text,
                            Vec::new(),
                            "XML".to_string(),
                            tree.root_node().range(),
                        );
                    } else if let Some(doc) = data.documents.get_mut(&url) {
                        doc.version = Some(version);
                    }
                }
            }
            return;
        } else if file_type == FileType::Routine || file_type == FileType::Cls {
            let is_rtn = if file_type == FileType::Routine {
                true
            } else {
                false
            };

            let Some((class_range, class_name)) =
                get_member_name_and_range_from_root(&text, tree.root_node(), is_rtn)
            else {
                eprintln!(
                    "Error: Failed to get name from root node for file url: {:?}",
                    url.path()
                );
                return;
            };
            // Commit INSIDE one lock
            let mut data = self.data.write();

            let existing_snapshot = data
                .documents
                .get(&url)
                .map(|d| (d.content.clone(), d.file_type.clone()));

            match existing_snapshot {
                None => {
                    let class_id = data.global_semantic_model.next_id();
                    data.add_document(
                        url.clone(),
                        &text,
                        &tree,
                        file_type,
                        Some(ClassId(class_id)),
                        class_name,
                        Some(version),
                        class_range,
                    );
                    // build override index/calls/vars for new doc too
                    // data.build_inheritance_and_variables(Some(url), Vec::new());
                }
                Some((old_text, old_type)) => {
                    if old_text != text || old_type != file_type {
                        data.incremental_update_document(
                            url,
                            &tree,
                            file_type,
                            version,
                            &text,
                            Vec::new(),
                            class_name,
                            class_range,
                        );
                    } else {
                        if let Some(doc) = data.documents.get_mut(&url) {
                            doc.version = Some(version);
                        }
                    }
                }
            }
        }
    }

    /// Wrapper to read document info from the inner `ProjectData`.
    pub fn get_document_info(&self, url: &Url) -> Option<(FileType, String, i32, Tree)> {
        self.data.read().get_document_info(url)
    }

    /// Wrapper to update a document inside the inner `ProjectData`
    pub fn update_document(
        &self,
        url: Url,
        tree: &Tree,
        file_type: FileType,
        version: i32,
        content: &str,
        changed_ranges: Vec<Range>,
    ) {
        let is_rtn = if file_type == FileType::Routine {
            true
        } else {
            false
        };
        let (class_range, class_name) = if file_type == FileType::Xml {
            (tree.root_node().range(), "XML".to_string())
        } else {
            if let Some((class_range, class_name)) =
                get_member_name_and_range_from_root(content, tree.root_node(), is_rtn)
            {
                (class_range, class_name)
            } else {
                eprintln!(
                    "Error: Failed to get name from root node for file url: {:?}",
                    url.path()
                );
                return;
            }
        };
        self.data.write().incremental_update_document(
            url,
            tree,
            file_type,
            version,
            content,
            changed_ranges,
            class_name,
            class_range,
        );
    }

    /// Wrapper to refactor a document inside the inner `ProjectData`
    pub fn refactor_document(&self, url: &Url, refactor_level: RefactorLevel) -> Option<String> {
        self.data.read().refactor_document(url, refactor_level)
    }

    /// Wrapper to refactor a workspace inside the inner `ProjectData`
    pub fn refactor(&self, refactor_level: RefactorLevel) -> Vec<(String, Url)> {
        self.data.read().refactor(refactor_level)
    }

    /// Return the project root path, if initialized.
    pub fn root_path(&self) -> Option<&std::path::Path> {
        self.project_root_path.get().and_then(|o| o.as_deref())
    }
}
