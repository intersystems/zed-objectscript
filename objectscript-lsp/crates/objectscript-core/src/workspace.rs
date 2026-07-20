use crate::common::{
    find_class_definition, generic_exit_statements, get_identifier_from_method_arg,
    get_member_name_from_root, get_node_children, get_routine_range, get_string_at_byte_range,
    initial_build_scope_tree, parse_line_ref, point_to_byte,
};
use crate::config::Config;
use crate::dependency_tracker::{DependencyGraph, Dependents};
use crate::document::Document;
use crate::global_semantic::GlobalSemanticModel;
use crate::local_semantic::LocalSemanticModel;

use crate::override_index::OverrideIndex;
use crate::parse_structures::{
    Class, ClassId, FileType, Language, MethodId, MethodRef, RefactorLevel, VariableRef,
};
use crate::refactor::{
    refactor_conditionals, refactor_for_statements, refactor_legacy_do_statements,
};

use crate::scope_structures::{MethodGlobalSymbol, ScopeId};
use crate::scope_tree::ScopeTree;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::OnceLock;
use tower_lsp::lsp_types::Url;
use tree_sitter::{
    Language as TsLanguage, Node, Parser, Point, Query, QueryCursor, Range, StreamingIterator, Tree,
};
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
    // /// Maps Class Name -> another hashmap which maps Method Name -> MethodGlobalSymbolId for all public methods
    pub method_defs: HashMap<String, HashMap<String, MethodRef>>,
    /// Maps Var Name -> another hashmap which maps MethodRef -> Vec<VariableRef> for that variable.
    pub pub_var_defs: HashMap<String, HashMap<MethodRef, HashMap<ScopeId, Vec<VariableRef>>>>,
    /// Holds the OverrideIndex for the workspace.
    pub override_index: OverrideIndex,
    /// Reverse inheritance index used by hierarchy-aware public variable lookup.
    pub dependent_class_index: Dependents,
    /// Graph of all calls to methods/procedures/subroutines for each class
    pub dependency_graph: DependencyGraph,
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
        tree: Tree,
        filetype: FileType,
        class_name: Option<String>,
        version: Option<i32>,
    ) -> bool {
        if self.documents.contains_key(&url) {
            eprintln!("Document already exists for file at :{:?}", url.path());
            return true;
        }
        self.add_document(url, code, tree, filetype, class_name, version);
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

    /// Parse and register a new document, initializing semantic + symbol state for `.cls` files.
    ///
    /// For class files (`FileType::Cls`), this:
    /// - Extracts the class definition/range
    /// - Builds an initial `Class` and method list from the tree-sitter tree
    /// - Creates a `ClassGlobalSymbol`, `ScopeTree`, and `Document`
    /// - Adds public methods into the global semantic model and method symbol tables
    /// - Adds private methods into the local semantic model and scope tree symbols
    /// - Registers class ids and local semantic model ids for later rebuilds
    ///
    /// Non-CLS file types are currently ignored by this function.
    pub fn add_document(
        &mut self,
        url: Url,
        code: String,
        tree: Tree,
        filetype: FileType,
        class_name: Option<String>,
        version: Option<i32>,
    ) {
        if filetype == FileType::Xml {
            let document = Document::new(code, tree, filetype, None, ScopeTree::new(None), version);
            self.documents.insert(url, document);
            return;
        } else if filetype == FileType::Routine || filetype == FileType::Cls {
            let Some(member_name) = class_name else {
                eprintln!(
                    "Error: missing class name while adding cls document for url: {}",
                    url.path()
                );
                return;
            };
            let content = code.as_str();
            let mut local_semantic_model = LocalSemanticModel::new();
            let is_rtn = if filetype == FileType::Routine {
                true
            } else {
                false
            };
            let mut class = Class::new(member_name.clone(), is_rtn);
            let starting_node = if is_rtn {
                tree.root_node()
            } else {
                let Some(node) = find_class_definition(tree.root_node()) else {
                    eprintln!(
                        "Error: Failed to find class definition for class named {:?}",
                        member_name
                    );
                    return;
                };
                node
            };
            let cls_range;
            if is_rtn {
                if let Some(rtn_range) = get_routine_range(tree.root_node()) {
                    cls_range = rtn_range;
                } else {
                    cls_range = starting_node.range();
                }
            } else {
                cls_range = starting_node.range();
            };
            let methods = class.initial_build(starting_node, content, is_rtn);

            let cls_id = ClassId(self.global_semantic_model.next_id());
            let scope_tree = initial_build_scope_tree(tree.clone(), cls_id, content, is_rtn);
            let mut document = Document::new(
                code,
                tree,
                filetype,
                Some(member_name.clone()),
                scope_tree,
                version,
            );
            // class id dne yet, because it gets added after. instead, we can just create the method ids here
            for (method, method_range, curr_method_id) in methods {
                let method_name = method.name.clone();
                let method_id = MethodId(curr_method_id);
                let method_ref = MethodRef {
                    class: cls_id,
                    id: method_id,
                    offset: None,
                };
                if method.is_public {
                    // add method to global semantic model

                    self.dependency_graph.get_or_add_node(method_ref);
                    // add methodId to class public methods field
                    class.methods.insert(method_name.clone(), method_ref);
                    // creates method global symbol in global semantic model
                    self.global_semantic_model.new_method_symbol(
                        method_name.clone(),
                        method_range,
                        url.clone(),
                        method_ref,
                    );
                    self.global_semantic_model.new_method(method, method_ref);
                    // add method symbol
                    self.method_defs
                        .entry(member_name.clone())
                        .or_insert_with(HashMap::new)
                        .insert(method_name.clone(), method_ref);
                } else {
                    // add method to local semantic model
                    local_semantic_model.new_method(method, method_ref);
                    self.dependency_graph.get_or_add_node(method_ref);
                    // add methodId to class private methods field
                    class.methods.insert(method_name.clone(), method_ref);
                    // find current scope and build symbol and add it to the scope
                    // this creates the symbol and adds the symbol id to the scope tree
                    document.scope_tree.new_method_symbol(
                        method_name.clone(),
                        method_range,
                        method_ref,
                    );
                    self.method_defs
                        .entry(member_name.clone())
                        .or_insert_with(HashMap::new)
                        .insert(method_name.clone(), method_ref);
                }
            }
            // add class to global semantic model
            self.global_semantic_model.new_class(class, cls_id);
            self.global_semantic_model.new_class_symbol(
                member_name.clone(),
                cls_range,
                url.clone(),
                cls_id,
            );
            // add class id corresponding to class struct
            self.classes.insert(member_name.clone(), cls_id);

            self.global_semantic_model
                .new_local_semantic(cls_id, local_semantic_model);
            // this creates the symbol and adds the symbol id to the scope tree
            document.class_id = Some(cls_id);
            self.documents.insert(url.clone(), document);
        }
    }

    /// Update a tracked document after text edits or reparse.
    ///
    /// This function:
    /// - Re-parses/derives the current class name from the new `tree` + `content`
    /// - Rebuilds the document's scope tree
    /// - Clears old symbol/semantic state for the document (class/method/variable symbols, local model)
    /// - Rebuilds class + method headers into semantic models (`rebuild_semantics`)
    /// - Updates the stored `Document` fields (content/tree/version/type/name)
    /// - Recomputes imports, inheritance, overrides, calls, and variables for the project
    pub fn update_document(
        &mut self,
        url: Url,
        tree: Tree,
        file_type: FileType,
        version: i32,
        content: &str,
    ) {
        if file_type == FileType::Xml {
            let Some(document) = self.get_document_mut(&url) else {
                generic_exit_statements("ProjectData", "update_document");
                return;
            };
            document.version = Some(version);
            document.file_type = file_type;
            document.tree = tree;
            document.content = content.to_string();
            document.class_name = None;
            document.class_id = None;
            document.scope_tree = ScopeTree::new(None);
            return;
        }

        if file_type == FileType::Routine || file_type == FileType::Cls {
            // a routine will be represented as a class in the workspace
            let is_rtn = if file_type == FileType::Routine {
                true
            } else {
                false
            };

            let Some(member_name) = get_member_name_from_root(content, tree.root_node(), is_rtn)
            else {
                eprintln!(
                    "Error: Failed to get name from root node for file url: {:?}",
                    url.path()
                );
                return;
            };
            let (cls_id, old_member_name) = {
                let Some(doc) = self.get_document(&url) else {
                    eprintln!(
                        "Error: Document for url {:?} DNE aborting update_document",
                        url.path()
                    );
                    return;
                };
                let Some(cls_id) = doc.class_id else {
                    eprintln!(
                        "Error: Class ID for document {:?} DNE aborting update_document",
                        doc
                    );
                    return;
                };
                let Some(old_member_name) = doc.class_name.clone() else {
                    eprintln!(
                        "Error: Name for document {:?} DNE aborting update_document",
                        doc
                    );
                    return;
                };
                (cls_id, old_member_name)
            };

            let mut old_methods: Vec<MethodRef> = Vec::new();
            if let Some(old_class) = self.global_semantic_model.get_class(&cls_id) {
                old_methods = old_class.methods.values().cloned().collect();
            }

            {
                let Some(doc) = self.get_document_mut(&url) else {
                    generic_exit_statements("ProjectData", "update_document");
                    return;
                };
                doc.scope_tree = initial_build_scope_tree(tree.clone(), cls_id, content, is_rtn);
            }
            // TODO: Make this incremental
            self.global_semantic_model
                .remove_document_symbols(&cls_id, &old_methods);
            self.global_semantic_model
                .reset_doc_semantics(&cls_id, member_name.clone());
            self.method_defs.remove(&old_member_name);
            self.classes.remove(&old_member_name);
            self.classes.insert(member_name.clone(), cls_id);
            for (_, class_map) in &mut self.pub_var_defs {
                for method_ref in &old_methods {
                    if class_map.contains_key(method_ref) {
                        class_map.remove(method_ref);
                    }
                }
            }

            let starting_node = if file_type == FileType::Routine {
                tree.root_node()
            } else {
                let Some(node) = find_class_definition(tree.root_node()) else {
                    eprintln!(
                        "Error: Failed to find class definition for class named {:?}",
                        member_name
                    );
                    return;
                };
                node
            };

            self.rebuild_semantics(
                url.clone(),
                starting_node,
                content,
                cls_id,
                member_name.clone(),
                file_type.clone(),
            );

            {
                let Some(document) = self.get_document_mut(&url) else {
                    generic_exit_statements("ProjectData", "update_document");
                    return;
                };
                document.version = Some(version);
                document.file_type = file_type;
                document.tree = tree;
                document.content = content.to_string();
                document.class_name = Some(member_name);
                document.class_id = Some(cls_id);
            }

            self.build_inheritance_and_variables(Some(url), Vec::new());
            return;
        }
    }

    /// Rebuild class + method header semantics for a document after a reparse.
    ///
    /// This reconstructs the `Class` for `class_id` from the given class definition `node` (for .cls) or root `node` (for routines), then:
    /// - Updates the class symbol (name/range/url)
    /// - Re-registers public methods and method symbols into the global semantic model
    /// - Re-registers private methods into the local semantic model and scope tree
    /// - Replaces the class slot in the global semantic model at `class_id`
    ///
    /// Note: This function does not rebuild statement-level variables/calls; those are handled by
    /// `build_inheritance_and_variables`.
    pub fn rebuild_semantics(
        &mut self,
        url: Url,
        node: Node,
        content: &str,
        class_id: ClassId,
        class_name: String,
        file_type: FileType,
    ) {
        let is_rtn = if file_type == FileType::Routine {
            true
        } else {
            false
        };
        // build vec of public methods to add to gsm at the end
        let mut class = Class::new(class_name.clone(), is_rtn);
        let methods = class.initial_build(node, content, is_rtn);
        self.global_semantic_model.update_class_symbol(
            class_name.clone(),
            node.range(),
            url.clone(),
            &class_id,
        );
        // class id dne yet, because it gets added after. instead, we can just create the method ids here
        for (method, method_range, id) in methods {
            let method_name = method.name.clone();
            let method_id = MethodId(id);
            let method_ref = MethodRef {
                class: class_id,
                id: method_id,
                offset: None,
            };
            if method.is_public {
                self.global_semantic_model.new_method(method, method_ref);
                self.global_semantic_model.new_method_symbol(
                    method_name.clone(),
                    method_range,
                    url.clone(),
                    method_ref,
                );
                // add method symbol
                self.method_defs
                    .entry(class_name.clone())
                    .or_insert_with(HashMap::new)
                    .insert(method_name.clone(), method_ref);
                // add methodId to class public methods field
                class.methods.insert(method_name.clone(), method_ref);
            } else {
                // add method to local semantic model
                let Some(lsm) = self.global_semantic_model.get_local_semantic_mut(&class_id) else {
                    eprintln!("Error: Failed to get local semantic model from gsm");
                    continue;
                };
                lsm.active = true;
                lsm.new_method(method, method_ref);
                // add methodId to class private methods field
                class.methods.insert(method_name.clone(), method_ref);
                // find current scope and build symbol and add it to the scope
                let Some(document) = self.get_document_mut(&url) else {
                    return;
                };
                document.scope_tree.new_method_symbol(
                    method_name.clone(),
                    method_range,
                    method_ref,
                );
                self.method_defs
                    .entry(class_name.clone())
                    .or_insert_with(HashMap::new)
                    .insert(method_name.clone(), method_ref);
            }
        }
        self.global_semantic_model.classes.insert(class_id, class);
        let Some(doc) = self.get_document_mut(&url) else {
            return;
        };
        doc.class_id = Some(class_id);
    }

    /// Compute imports, inheritance, override resolution, call sites, and variable symbols.
    ///
    /// If `only` is provided, only that document is scanned for import/extends changes; the
    /// inheritance/override index is still rebuilt globally, and method calls/variables are
    /// recomputed for all classes in the semantic model.
    pub fn build_inheritance_and_variables(&mut self, only: Option<Url>, exclude: Vec<Url>) {
        let mut indices_to_exclude = Vec::new();
        if let Some(url) = only {
            if exclude.contains(&url) {
                eprintln!(
                    "Error: Cannot specify the same URL in both only and exclude fields, aborting build_inheritance_and_variables"
                );
                return;
            }

            let document_class_id = {
                let Some(document) = self.documents.get(&url) else {
                    eprintln!(
                        "Error: Failed to get document for url {:?}, aborting build_inheritance_and_variables",
                        url.path()
                    );
                    return;
                };
                let Some(class_id) = document.class_id else {
                    eprintln!(
                        "Error: Failed to get class id for url {:?}, aborting build_inheritance_and_variables",
                        url.path()
                    );
                    return;
                };
                class_id
            };

            // Snapshot inherited_classes before recomputing extends, so we can
            // detect which classes got newly resolved parents.
            let old_inherited: HashMap<ClassId, Vec<ClassId>> = self
                .global_semantic_model
                .classes
                .iter()
                .map(|(&id, c)| (id, c.inherited_classes.clone()))
                .collect();

            // Recompute extends/imports for ALL classes so the override index
            // sees correct inherited_classes even when documents were added out
            // of order (e.g. subclass opened before its superclass was indexed).
            let all_urls: Vec<Url> = self.documents.keys().cloned().collect();
            for doc_url in &all_urls {
                let Some(document) = self.documents.get(doc_url) else {
                    continue;
                };
                let Some(cls_id) = document.class_id else {
                    continue;
                };
                if document.file_type == FileType::Routine {
                    continue;
                }
                let doc_tree = document.tree.clone();
                let doc_content = document.content.clone();
                self.recompute_imports_for_url(&doc_tree, doc_content.as_str(), &cls_id);
                self.recompute_extends_for_url(&doc_tree, doc_content.as_str(), &cls_id);
            }

            // Any class whose inherited_classes changed needs variable/call
            // rebuilding too — not just the target document.
            let mut changed_class_ids: Vec<usize> = Vec::new();
            for (&cls_id, class) in &self.global_semantic_model.classes {
                let old = old_inherited
                    .get(&cls_id)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                if old != class.inherited_classes.as_slice() {
                    changed_class_ids.push(cls_id.0);
                }
            }

            indices_to_exclude = self
                .classes
                .values()
                .filter(|&class_id| {
                    class_id != &document_class_id && !changed_class_ids.contains(&class_id.0)
                })
                .map(|class_id| class_id.0)
                .collect();
        } else {
            let urls: Vec<Url> = self
                .documents
                .keys()
                .cloned()
                .into_iter()
                .filter(|url| !exclude.contains(url))
                .collect();
            for url in &urls {
                let (document_file_type, document_class_id, doc_tree, doc_content) = {
                    let Some(document) = self.documents.get(&url) else {
                        eprintln!(
                            "Error: Failed to get document for url {:?}, aborting build_inheritance_and_variables",
                            url.path()
                        );
                        return;
                    };
                    let Some(class_id) = document.class_id else {
                        eprintln!(
                            "Error: Failed to get class id for url {:?}, aborting build_inheritance_and_variables",
                            url.path()
                        );
                        return;
                    };
                    (
                        document.file_type.clone(),
                        class_id,
                        document.tree.clone(),
                        document.content.clone(),
                    )
                };
                let is_rtn = if document_file_type == FileType::Routine {
                    true
                } else {
                    false
                };
                if !is_rtn {
                    self.recompute_imports_for_url(
                        &doc_tree,
                        doc_content.as_str(),
                        &document_class_id,
                    );
                    self.recompute_extends_for_url(
                        &doc_tree,
                        doc_content.as_str(),
                        &document_class_id,
                    );
                }
            }
            for url in &exclude {
                let Some(document) = self.documents.get(url) else {
                    eprintln!("Error: Document DNE for url {:?}", url.path());
                    continue;
                };
                if let Some(index) = document.class_id {
                    indices_to_exclude.push(index.0);
                }
            }
        }
        // Recompute inheritance + override index
        self.global_semantic_model.class_keyword_inheritance();
        // currently tracks superclass -> all subclasses that are dependent on it
        self.dependent_class_index = self.global_semantic_model.build_dependents();
        let idx = self.global_semantic_model.build_override_index();
        self.override_index = idx.clone();

        // TODO: need to calculate which classes to actually rebuild semantics for
        let class_indices: Vec<ClassId> = self.classes.values().cloned().collect();
        // Class name, method name -> Vec<orefs>
        for class_index in class_indices {
            if indices_to_exclude.contains(&class_index.0) {
                continue;
            }
            let (class_name, methods, is_procedure_block, default_language) = {
                let Some(class) = self.global_semantic_model.get_class(&class_index) else {
                    continue;
                };
                let is_procedure_block = if class.is_procedure_block.is_none() {
                    false
                } else {
                    class.is_procedure_block.unwrap()
                };
                let default_language = if class.default_language.is_none() {
                    Language::Objectscript
                } else {
                    class.default_language.clone().unwrap()
                };
                let methods = class.methods.clone();
                (
                    class.name.clone(),
                    methods,
                    is_procedure_block,
                    default_language,
                )
            };

            let url = {
                let Some(class_global_symbol) =
                    self.global_semantic_model.get_class_symbol(&class_index)
                else {
                    eprintln!("Error: Class Symbol DNE for class named {:?}", &class_name);
                    continue;
                };
                class_global_symbol.url.clone()
            };
            let (content, tree, scope_tree_snapshot, file_type) = {
                let Some(document) = self.get_document(&url) else {
                    eprintln!(
                        "Error: Document DNE for class named {:?} skipping this class in build_inheritance_and_variables",
                        &class_name
                    );
                    continue;
                };
                let content = document.content.clone();
                let tree = document.tree.clone();
                let scope_tree_snapshot = document.scope_tree.clone();
                let file_type = document.file_type.clone();
                (content, tree, scope_tree_snapshot, file_type)
            };
            let content = content.as_str();
            let tree_root_node = tree.root_node();
            let is_rtn = if file_type == FileType::Routine {
                true
            } else {
                false
            };
            let language: TsLanguage;
            if is_rtn {
                language = LANGUAGE_OBJECTSCRIPT_ROUTINE.into();
            } else {
                language = LANGUAGE_OBJECTSCRIPT_UDL.into();
            };
            // ---------- public methods ----------
            for (method_name, method_ref) in methods {
                // inherit class keywords if not explicitly assigned
                if !is_rtn {
                    let method =
                        if let Some(m) = self.global_semantic_model.get_mut_method(&method_ref) {
                            m
                        } else if let Some(lsm) = self
                            .global_semantic_model
                            .get_local_semantic_mut(&class_index)
                            && let Some(m) = lsm.get_method_mut(&method_ref)
                        {
                            m
                        } else {
                            continue;
                        };
                    method.update_keywords(is_procedure_block, default_language.clone());
                }

                let loc = if let Some(s) =
                    self.get_public_method_symbol(class_name.as_str(), method_name.as_str())
                {
                    s.location
                } else if let Some(s) =
                    scope_tree_snapshot.get_private_method_symbol(&method_ref.id)
                {
                    s.location
                } else {
                    continue;
                };

                let Some(method_definition_node) =
                    tree_root_node.named_descendant_for_byte_range(loc.start_byte, loc.end_byte)
                else {
                    continue;
                };

                {
                    self.find_method_dependencies(
                        method_definition_node,
                        content,
                        &language,
                        class_name.as_str(),
                        &method_ref,
                    );
                }

                // Variables: compute first (immutable), then apply (mutable) to avoid long borrows
                let var_results = {
                    let method = if let Some(m) = self.global_semantic_model.get_method(&method_ref)
                    {
                        m
                    } else if let Some(lsm) =
                        self.global_semantic_model.get_local_semantic(&class_index)
                        && let Some(m) = lsm.get_method(&method_ref)
                    {
                        m
                    } else {
                        continue;
                    };
                    method.build_variables(method_definition_node, content, is_rtn)
                };

                for (variable, variable_range, refs_to_other_vars) in var_results {
                    let var_name = variable.name.clone();
                    let var_is_public = variable.is_public;

                    if refs_to_other_vars.contains(&var_name) {
                        continue;
                    }
                    let Some(scope_id) =
                        scope_tree_snapshot.find_current_scope(variable_range.start_point)
                    else {
                        eprintln!(
                            "Error: failed to find scope for variable range, skipping (build_inheritance_and_variables)"
                        );
                        continue;
                    };
                    if var_is_public {
                        let variable_ref = self
                            .global_semantic_model
                            .new_variable(variable, method_ref, scope_id);
                        {
                            let method = if let Some(m) =
                                self.global_semantic_model.get_mut_method(&method_ref)
                            {
                                m
                            } else if let Some(lsm) = self
                                .global_semantic_model
                                .get_local_semantic_mut(&class_index)
                                && let Some(m) = lsm.get_method_mut(&method_ref)
                            {
                                m
                            } else {
                                continue;
                            };
                            method
                                .variables
                                .entry(var_name.clone())
                                .or_insert_with(Vec::new)
                                .push((variable_ref, scope_id));
                        }

                        self.global_semantic_model.new_variable_symbol(
                            variable_range,
                            url.clone(),
                            refs_to_other_vars.clone(),
                            method_ref,
                            variable_ref,
                            scope_id,
                        );

                        {
                            let Some(document) = self.get_document_mut(&url) else {
                                continue;
                            };
                            document.scope_tree.new_public_var_symbol(
                                var_name.clone(),
                                variable_range,
                                variable_ref,
                            );
                        }
                        self.pub_var_defs
                            .entry(var_name.clone())
                            .or_insert_with(HashMap::new)
                            .entry(method_ref.clone())
                            .or_insert_with(HashMap::new)
                            .entry(scope_id)
                            .or_insert_with(Vec::new)
                            .push(variable_ref);
                    } else {
                        let variable_ref = {
                            let Some(lsm) = self
                                .global_semantic_model
                                .get_local_semantic_mut(&class_index)
                            else {
                                continue;
                            };
                            lsm.new_variable(method_ref, variable, scope_id)
                        };

                        {
                            let method = if let Some(m) =
                                self.global_semantic_model.get_mut_method(&method_ref)
                            {
                                m
                            } else if let Some(lsm) = self
                                .global_semantic_model
                                .get_local_semantic_mut(&class_index)
                                && let Some(m) = lsm.get_method_mut(&method_ref)
                            {
                                m
                            } else {
                                continue;
                            };
                            method
                                .variables
                                .entry(var_name.clone())
                                .or_insert_with(Vec::new)
                                .push((variable_ref, scope_id));
                        }

                        {
                            let Some(document) = self.get_document_mut(&url) else {
                                continue;
                            };
                            document.scope_tree.new_variable_symbol(
                                var_name.clone(),
                                variable_range,
                                refs_to_other_vars,
                                variable_ref,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Recomputes the import list for the class defined in `url`.
    ///
    /// This scans the non-class-definition portion of the file (everything before the
    /// trailing `class_definition` node) for `import_code` statements, resolves imported
    /// class names to `ClassId`s using `self.classes`, and updates the corresponding
    /// `Class.imports` entry in the global semantic model.
    ///
    /// If the document or owning class cannot be found, the function logs a warning and
    /// returns early without modifying state.
    fn recompute_imports_for_url(&mut self, tree: &Tree, content: &str, class_id: &ClassId) {
        let source_file_children = get_node_children(tree.root_node());
        let mut imports = Vec::new();
        for class_child in source_file_children {
            if class_child.kind() == "import_code" {
                let import_code_children = get_node_children(class_child);
                for import_child in import_code_children {
                    if import_child.kind() == "class_name" {
                        let Some(identifier) = import_child.named_child(0) else {
                            eprintln!(
                                "Error: class name child should exist at index 0, must update parsing in recompute_imports_for_url"
                            );
                            continue;
                        };
                        let Some(name) = get_string_at_byte_range(content, identifier.byte_range())
                        else {
                            continue;
                        };
                        if let Some(id) = self.classes.get(&name).copied() {
                            imports.push(id);
                        }
                    }
                }
            }
        }
        if let Some(class) = self.global_semantic_model.classes.get_mut(class_id) {
            class.imports = imports;
        }
    }

    /// Recompute direct `extends` (inheritance) dependencies for the class defined in `url`.
    ///
    /// Parses the class definition's `class_extends` entries and updates `class.inherited_classes`
    /// with direct parent `ClassId`s (when resolvable). This should be run before building the
    /// override index, which assumes direct parents only.
    fn recompute_extends_for_url(&mut self, tree: &Tree, content: &str, class_id: &ClassId) {
        let mut inherited = Vec::new();
        let Some(node) = find_class_definition(tree.root_node()) else {
            eprintln!(
                "Error: Failed to find class definition for class, exiting recompute_extends_for_url",
            );
            return;
        };

        let Some(possible_extends_node) = node.named_child(2) else {
            eprintln!(
                "Error: class definition node should always  have a child at index 2, parsing error, fix recompute_extends_for_url"
            );
            return;
        };

        if possible_extends_node.kind() == "class_extends" {
            let class_extends_children = get_node_children(possible_extends_node);
            for class_extends_child in class_extends_children {
                if class_extends_child.kind() == "class_name" {
                    let Some(identifier) = class_extends_child.named_child(0) else {
                        eprintln!(
                            "Error: class name child should exist at index 0, must update parsing in recompute_imports_for_url"
                        );
                        continue;
                    };
                    let Some(name) = get_string_at_byte_range(content, identifier.byte_range())
                    else {
                        continue;
                    };
                    if let Some(id) = self.classes.get(&name).copied() {
                        inherited.push(id);
                    }
                }
            }
        }
        if let Some(class) = self.global_semantic_model.classes.get_mut(class_id) {
            class.inherited_classes = inherited;
        }
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
        let Some(document) = self.documents.get_mut(url) else {
            eprintln!("Error: Couldn't find document for url: {}", url.path());
            return None;
        };
        Some(document)
    }

    /// Lookup the global symbol (name/range/url) for a public method in a class.
    ///
    /// This first resolves the method's symbol id from `pub_method_defs[class_name][method_name]`,
    /// then retrieves the `MethodGlobalSymbol` from the global semantic model.
    fn get_public_method_symbol(
        &self,
        class_name: &str,
        method_name: &str,
    ) -> Option<&MethodGlobalSymbol> {
        let Some(&sym_id) = self
            .method_defs
            .get(class_name)
            .and_then(|m| m.get(method_name))
        else {
            return None;
        };

        self.global_semantic_model.get_method_symbol(&sym_id)
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
                cls_doc.scope_tree.get_private_method_symbol(&method_ref.id)
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

        let Some(class_name) = document.class_name.clone() else {
            return locations;
        };

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
                scope_children.push(var_ref_scope_id);
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
            scope_children.push(var_ref_scope_id);
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
                        let Some(variable_refs_hash_map) =
                            public_var_definitions.get(ancestor_ref)
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
                        scope_children.push(method_scope_id);
                        for (child_scope_id, variable_refs) in variable_refs_hash_map {
                            if !scope_children.contains(child_scope_id) {
                                continue;
                            }
                            for variable_ref in variable_refs {
                                if let Some(var_id) = variable_ref.pub_id
                                    && let Some(symbol) =
                                        self.global_semantic_model.get_variable_symbol(
                                            ancestor_ref,
                                            var_id.0,
                                            child_scope_id,
                                        )
                                {
                                    if symbol.location.end_byte < method_call_range.start_byte {
                                        if !seen_scope_ids.contains(child_scope_id) {
                                            let index = locations.len();
                                            locations
                                                .push((symbol.url.clone(), symbol.location));
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
        let (method_refs, locations) =
            self.find_classes_from_oref(oref_name, oref_method_name, curr_class, oref_ref_range);
        if !resolve_method {
            return locations;
        }
        let mut oref_method_locations = Vec::new();
        for (method_ref, _) in method_refs {
            oref_method_locations.extend(self.get_method_definition(&method_ref, None))
        }
        oref_method_locations
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

    /// Returns the location of the superclass method that the given subclass method overrides
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
            let superclass_method_ref = match self.override_index.overrides.get(&method_ref) {
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
            for inherited_class_id in &class.inherited_classes {
                let Some(inherited_class) =
                    self.global_semantic_model.get_class(inherited_class_id)
                else {
                    eprintln!("Error: Inherited Class struct DNE, skipping",);
                    continue;
                };
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
            }
        }
        locations
    }

    /// Return locations of methods that override a given public method.
    ///
    /// Looks up the current document's class, confirms `method_name` is a public method, then uses
    /// `override_index.overridden_by` to find overriding methods (public or private) in subclasses.
    ///
    /// Each returned `(Url, Range)` points to the overriding method's definition location.
    pub fn get_method_overrides(&self, method_ref: &MethodRef) -> Vec<(Url, Range)> {
        let mut locations = Vec::new();
        // ---- overridden-by list ----
        let overrides = match self.override_index.overridden_by.get(method_ref) {
            Some(v) => v,
            None => {
                return locations;
            }
        };
        for override_method_ref in overrides {
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
                    .get_private_method_symbol(&override_method_ref.id)
            {
                locations.push((override_cls_url.clone(), sym.location));
            }
        }
        locations
    }

    /// Finds all potential variable references for `oref_name`,
    /// and finds the corresponding variable and checks if it is an oref.
    /// If yes, finds the method and class that the oref is referencing, and
    /// creates a method ref. If there is another oref definition in the same scope
    /// and it comes after the first definition, it will replace the first definition.
    /// If the definition is defined in the current class and method, then if the
    /// definition comes after `point` it is not added. Similarly, if the definition
    /// is from another class/method (and is connected by method calls, tracked in dependencyGraph),
    /// if the definition comes after the method call that connects it, it is not added.
    ///
    /// The first Vec returned contains:
    /// - `MethodRef` - a reference to the actual method and class that the oref method call was referencing.
    /// - `Range` - the oref method call node range
    /// The second Vec returned contains:
    /// - `Url` - the url of the associated variable definition (where the oref was created)
    /// - `Range` - the range of the associated variable definition (where the oref was created)
    fn find_classes_from_oref(
        &self,
        oref_name: &str,
        oref_method_name: &str,
        curr_class: &str,
        original_method_call: Range, // this is a range within the current method
    ) -> (Vec<(MethodRef, Range)>, Vec<(Url, Range)>) {
        let point = original_method_call.start_point;
        let mut oref_method_refs = Vec::new();
        let mut locations = Vec::new();
        let curr_class_url = if let Some(class_id) = self.classes.get(curr_class)
            && let Some(class_sym) = self.global_semantic_model.get_class_symbol(&class_id)
        {
            &class_sym.url
        } else {
            return (oref_method_refs, locations);
        };
        let Some(current_document) = self.get_document(curr_class_url) else {
            eprintln!(
                "Error: failed to get document for file {:?}. Aborting find_classes_from_oref",
                curr_class_url.path()
            );
            return (oref_method_refs, locations);
        };
        let Some(curr_method_name) = current_document.scope_tree.get_method_name(point) else {
            return (oref_method_refs, locations);
        };

        let Some(var_ref_scope_id) = current_document.scope_tree.find_current_scope(point) else {
            eprintln!(
                "Error: failed to find scope for variable range, returning (find_classes_from_oref)"
            );
            return (oref_method_refs, locations);
        };

        if let Some(current_method_ref) = self
            .method_defs
            .get(curr_class)
            .and_then(|method_refs| method_refs.get(&curr_method_name))
        {
            let is_variable_public =
                self.is_variable_public(*current_method_ref, oref_name.to_string());
            let potential_oref_refs = current_document
                .scope_tree
                .get_oref_references(oref_name, var_ref_scope_id);
            if !potential_oref_refs.is_empty() {
                let Some(lsm) = self
                    .global_semantic_model
                    .get_local_semantic(&current_method_ref.class)
                else {
                    return (oref_method_refs, locations);
                };
                let mut location_hash = HashMap::new();
                let mut seen_scope_ids = Vec::new();
                let mut scope_children = current_document
                    .scope_tree
                    .get_scope_children(&var_ref_scope_id);
                scope_children.push(var_ref_scope_id);
                for (child_scope_id, variable_refs) in potential_oref_refs {
                    if !scope_children.contains(&child_scope_id) {
                        continue;
                    }
                    for variable_ref in variable_refs {
                        if !is_variable_public
                            && let Some(var_id) = variable_ref.priv_id
                            && let Some(variable) =
                                lsm.get_variable(current_method_ref, var_id.0, &child_scope_id)
                            && let Some(symbol) = current_document
                                .scope_tree
                                .get_variable_symbol(var_id.0, &child_scope_id)
                        {
                            if variable.is_oref
                                && let Some(oref_class_name) = variable.cls.clone()
                                && let Some(oref_method_ref) = self
                                    .method_defs
                                    .get(&oref_class_name)
                                    .and_then(|methods| methods.get(oref_method_name))
                            {
                                if symbol.location.end_point < point {
                                    if !seen_scope_ids.contains(&child_scope_id) {
                                        let index = oref_method_refs.len();
                                        oref_method_refs
                                            .push((*oref_method_ref, original_method_call));
                                        locations.push((curr_class_url.clone(), symbol.location));
                                        seen_scope_ids.push(child_scope_id);
                                        location_hash.insert(child_scope_id, index);
                                    } else if let Some(&index) = location_hash.get(&child_scope_id)
                                    {
                                        let curr_indexed_sym_range = locations[index].1;
                                        if curr_indexed_sym_range.end_byte
                                            < symbol.location.start_byte
                                        {
                                            oref_method_refs[index] =
                                                (*oref_method_ref, original_method_call);
                                            locations[index] =
                                                (curr_class_url.clone(), symbol.location);
                                        }
                                    }
                                }
                            }
                        } else if is_variable_public
                            && let Some(var_id) = variable_ref.pub_id
                            && let Some(variable) = self.global_semantic_model.get_variable(
                                current_method_ref,
                                var_id.0,
                                &child_scope_id,
                            )
                            && let Some(symbol) = self.global_semantic_model.get_variable_symbol(
                                current_method_ref,
                                var_id.0,
                                &child_scope_id,
                            )
                        {
                            if variable.is_oref
                                && let Some(oref_class_name) = variable.cls.clone()
                                && let Some(oref_method_ref) = self
                                    .method_defs
                                    .get(&oref_class_name)
                                    .and_then(|methods| methods.get(oref_method_name))
                            {
                                if symbol.location.end_point < point {
                                    if !seen_scope_ids.contains(&child_scope_id) {
                                        let index = oref_method_refs.len();
                                        oref_method_refs
                                            .push((*oref_method_ref, original_method_call));
                                        locations.push((symbol.url.clone(), symbol.location));
                                        seen_scope_ids.push(child_scope_id);
                                        location_hash.insert(child_scope_id, index);
                                    } else if let Some(&index) = location_hash.get(&child_scope_id)
                                    {
                                        let curr_indexed_sym_range = locations[index].1;
                                        if curr_indexed_sym_range.end_byte
                                            < symbol.location.start_byte
                                        {
                                            locations[index] =
                                                (symbol.url.clone(), symbol.location);
                                            oref_method_refs[index] =
                                                (*oref_method_ref, original_method_call);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if is_variable_public {
                if let Some(&node_index) = self.dependency_graph.get_node(*current_method_ref)
                    && let Some(public_var_definitions) = self.pub_var_defs.get(oref_name)
                {
                    let all_ancestors = self.dependency_graph.all_ancestors(node_index);
                    let mut found_depth: Option<usize> = None;
                    for (ancestor_ref, method_call_range, depth) in &all_ancestors {
                        if let Some(fd) = found_depth {
                            if *depth > fd {
                                break;
                            }
                        }
                        let Some(variable_refs_hash_map) =
                            public_var_definitions.get(ancestor_ref)
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
                        scope_children.push(method_scope_id);
                        for (child_scope_id, variable_refs) in variable_refs_hash_map {
                            if !scope_children.contains(child_scope_id) {
                                continue;
                            }
                            for variable_ref in variable_refs {
                                if let Some(var_id) = variable_ref.pub_id
                                    && let Some(symbol) =
                                        self.global_semantic_model.get_variable_symbol(
                                            ancestor_ref,
                                            var_id.0,
                                            child_scope_id,
                                        )
                                    && let Some(variable) = self
                                        .global_semantic_model
                                        .get_variable(ancestor_ref, var_id.0, child_scope_id)
                                    && variable.is_oref
                                    && let Some(oref_class_name) = variable.cls.clone()
                                    && let Some(oref_method_ref) = self
                                        .method_defs
                                        .get(&oref_class_name)
                                        .and_then(|methods| methods.get(oref_method_name))
                                {
                                    if symbol.location.end_byte < method_call_range.start_byte {
                                        if !seen_scope_ids.contains(child_scope_id) {
                                            let index = oref_method_refs.len();
                                            oref_method_refs
                                                .push((*oref_method_ref, original_method_call));
                                            locations
                                                .push((symbol.url.clone(), symbol.location));
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
                                                oref_method_refs[index] =
                                                    (*oref_method_ref, original_method_call);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if !oref_method_refs.is_empty() {
                            found_depth = Some(*depth);
                        }
                    }
                    if !oref_method_refs.is_empty() {
                        return (oref_method_refs, locations);
                    }
                }
            }
        }
        return (oref_method_refs, locations);
    }

    fn get_method_calls(
        &self,
        method_def_node: Node,
        content: &str,
        language: &TsLanguage,
        query_str: &str,
        curr_class: &str,
    ) -> Vec<(MethodRef, Range)> {
        let mut method_refs = Vec::new();
        if let Ok(query) = Query::new(language, query_str) {
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(&query, method_def_node, content.as_bytes());
            while let Some(query_match) = iter.next() {
                let matched_node = query_match.captures[0].node;
                match matched_node.kind() {
                    "class_method_call" => {
                        if let Some(class_ref) = matched_node.named_child(0)
                            && let Some(method_name_node) = matched_node.named_child(1)
                            && let Some(class_name_node) = class_ref.named_child(1)
                        {
                            // this part will remove the strings and such (it grabs the actual $.identifier node)
                            if let Some(method_name) = method_name_node.named_child(0)
                                && let Some(class_name) = class_name_node.named_child(0)
                            {
                                if let Some(method_name) =
                                    get_string_at_byte_range(content, method_name.byte_range())
                                    && let Some(class_name) =
                                        get_string_at_byte_range(content, class_name.byte_range())
                                {
                                    if let Some(method_ref) = self
                                        .method_defs
                                        .get(&class_name)
                                        .and_then(|method_refs| method_refs.get(&method_name))
                                    {
                                        method_refs.push((*method_ref, matched_node.range()));
                                    }
                                }
                            }
                        } else {
                            eprintln!(
                                "Error: Expected child at index 0 for class_method_call node, and expected child at index 1 for class_ref"
                            );
                        };
                    }
                    "system_defined_function" => {
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
                                // instance name is first for $method
                                let Some(oref_node) = matched_node.named_child(0) else {
                                    eprintln!(
                                        "Error: System Defined Variable has no child at index 0"
                                    );
                                    continue;
                                };
                                let Some(instance_name) =
                                    get_identifier_from_method_arg(oref_node, content)
                                else {
                                    continue;
                                };

                                let Some(method_name_node) = matched_node.named_child(1) else {
                                    eprintln!(
                                        "Error: System Defined Variable has no child at index 1"
                                    );
                                    continue;
                                };
                                if let Some(method_name) =
                                    get_identifier_from_method_arg(method_name_node, content)
                                {
                                    method_refs.extend(
                                        self.find_classes_from_oref(
                                            &instance_name,
                                            &method_name,
                                            &curr_class,
                                            oref_node.range(),
                                        )
                                        .0,
                                    );
                                } else {
                                    eprintln!("Error: Couldn't get method name from $CLASSMETHOD");
                                }
                            } else if func_name.eq_ignore_ascii_case("$classmethod")
                                || func_name.eq_ignore_ascii_case("$zobjclassmethod")
                            {
                                let method_node;
                                let class_name;

                                if method_args.trim_start().chars().next() == Some(',') {
                                    // class is current one
                                    method_node = matched_node.named_child(0);
                                    class_name = curr_class.to_string();
                                } else {
                                    method_node = matched_node.named_child(1);
                                    let Some(class_name_node) = matched_node.named_child(0) else {
                                        // this should be a method arg
                                        eprintln!(
                                            "Error: Expected system defined function to have a child at index 0"
                                        );
                                        continue;
                                    };
                                    let Some(cls_name) =
                                        get_identifier_from_method_arg(class_name_node, content)
                                    else {
                                        continue;
                                    };
                                    class_name = cls_name;
                                }
                                let Some(method_arg) = method_node else {
                                    continue;
                                };
                                if let Some(method_name) =
                                    get_identifier_from_method_arg(method_arg, content)
                                {
                                    if let Some(method_ref) = self
                                        .method_defs
                                        .get(&class_name)
                                        .and_then(|method_refs| method_refs.get(&method_name))
                                    {
                                        method_refs.push((*method_ref, matched_node.range()));
                                    }
                                }
                            } else if func_name.eq_ignore_ascii_case("$system") {
                                if let Some(class_name_node) = matched_node.named_child(0)
                                    && let Some(method_name_node) = matched_node.named_child(1)
                                {
                                    let Some(class_name) = get_string_at_byte_range(
                                        content,
                                        class_name_node.byte_range(),
                                    ) else {
                                        continue;
                                    };
                                    let Some(method_name) = get_string_at_byte_range(
                                        content,
                                        method_name_node.byte_range(),
                                    ) else {
                                        continue;
                                    };
                                    if let Some(method_ref) = self
                                        .method_defs
                                        .get(&class_name)
                                        .and_then(|method_refs| method_refs.get(&method_name))
                                    {
                                        method_refs.push((*method_ref, matched_node.range()));
                                    }
                                }
                            }
                        }
                    }
                    "relative_dot_method" => {
                        if let Some(oref_method) = matched_node.named_child(0)
                            && let Some(method_name_node) = oref_method.named_child(0)
                            && let Some(method_identifier) = method_name_node.named_child(0)
                        {
                            let Some(method_name) =
                                get_string_at_byte_range(content, method_identifier.byte_range())
                            else {
                                continue;
                            };
                            if let Some(method_ref) = self
                                .method_defs
                                .get(curr_class)
                                .and_then(|method_refs| method_refs.get(&method_name))
                            {
                                method_refs.push((*method_ref, matched_node.range()));
                            }
                        }
                    }
                    "routine_tag_call" | "goto_argument" | "print_argument" => {
                        let Some(routine_tag_call_child) = matched_node.named_child(0) else {
                            eprintln!(
                                "Error: routine tag call node should have a child at index 0, update parsing in get_method_calls"
                            );
                            continue;
                        };

                        match routine_tag_call_child.kind() {
                            "method_name" => {
                                // this version doesn't have wrapped in quotes option
                                let Some(method_name) =
                                    get_string_at_byte_range(content, matched_node.byte_range())
                                else {
                                    continue;
                                };
                                if let Some(method_ref) = self
                                    .method_defs
                                    .get(curr_class)
                                    .and_then(|method_refs| method_refs.get(&method_name))
                                {
                                    method_refs.push((*method_ref, matched_node.range()));
                                }
                            }
                            "line_ref" => {
                                let (method_name, routine_name, offset) = parse_line_ref(
                                    routine_tag_call_child,
                                    content,
                                    curr_class.to_string(),
                                );

                                if let Some(method_ref) = self
                                    .method_defs
                                    .get(&routine_name)
                                    .and_then(|method_refs| method_refs.get(&method_name))
                                {
                                    let method_ref = MethodRef {
                                        class: method_ref.class,
                                        id: method_ref.id,
                                        offset: offset,
                                    };
                                    method_refs.push((method_ref, matched_node.range()));
                                }
                            }
                            _ => continue,
                        }
                    }

                    _ => continue,
                }
            }
        }
        method_refs
    }

    // Parses a method definition node to extract it's dependencies for the given method.
    // Adds edges from the method to it's dependencies in the dependency graph
    fn find_method_dependencies(
        &mut self,
        node: Node,
        content: &str,
        language: &TsLanguage,
        curr_class: &str,
        method_ref: &MethodRef,
    ) {
        // Vec<Class name, Method name>
        // first, find all class method definitions
        let query_str = "(class_method_call) @classmethodcall";
        let mut method_refs = self.get_method_calls(node, content, language, query_str, curr_class);

        let query_str = "(system_defined_function) @systemfunc";

        method_refs.extend(self.get_method_calls(node, content, language, query_str, curr_class));

        let query_str = "(relative_dot_method) @relativemethod";
        method_refs.extend(self.get_method_calls(node, content, language, query_str, curr_class));

        let query_str = r#"[
            (routine_tag_call)
            (goto_argument)
            (print_argument)
            ] @routine "#;
        method_refs.extend(self.get_method_calls(node, content, language, query_str, curr_class));
        for (dep_method_ref, method_call_range) in method_refs {
            self.dependency_graph
                .add_edge(method_ref.clone(), dep_method_ref, method_call_range);
        }
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
                override_index: OverrideIndex::new(),
                dependent_class_index: Dependents::new(),
                dependency_graph: DependencyGraph::new(),
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
                    data.add_document(url, text, tree, file_type, None, Some(version));
                }
                Some((old_text, old_type)) => {
                    if old_text != text || old_type != file_type {
                        data.update_document(url, tree, file_type, version, &text);
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

            let Some(member_name) = get_member_name_from_root(&text, tree.root_node(), is_rtn)
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
                    data.add_document(
                        url.clone(),
                        text,
                        tree,
                        file_type,
                        Some(member_name),
                        Some(version),
                    );
                    // build override index/calls/vars for new doc too
                    data.build_inheritance_and_variables(Some(url), Vec::new());
                }
                Some((old_text, old_type)) => {
                    if old_text != text || old_type != file_type {
                        data.update_document(url, tree, file_type, version, &text);
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
        tree: Tree,
        file_type: FileType,
        version: i32,
        content: &str,
    ) {
        self.data
            .write()
            .update_document(url, tree, file_type, version, content);
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
