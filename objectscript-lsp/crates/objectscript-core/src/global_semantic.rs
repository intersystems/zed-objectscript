use crate::common::generic_exit_statements;
use crate::dependency_tracker::Dependents;
use crate::local_semantic::LocalSemanticModel;
use crate::override_index::OverrideIndex;
use crate::parse_structures::{
    Class, ClassId, DfsState, Language, Method, MethodRef, PublicVarId, Variable, VariableRef,
};
use crate::scope_structures::{
    ClassGlobalSymbol, MethodGlobalSymbol, ScopeId, VariableGlobalSymbol,
};
use std::collections::{HashMap, HashSet};
use tower_lsp::lsp_types::Url;
use tree_sitter::Range;

/// Holds the semantic information and symbols for classes, public methods, and public variables.
#[derive(Clone, Debug)]
pub struct GlobalSemanticModel {
    /// Stores public variables per class.
    pub variables: HashMap<MethodRef, HashMap<ScopeId, Vec<Variable>>>,
    /// Stores all classes in a workspace.
    pub classes: HashMap<ClassId, Class>,
    /// Stores public methods per class.
    pub methods: HashMap<MethodRef, Method>,
    /// Stores all local semantic models in a workspace.
    pub lsms: HashMap<ClassId, LocalSemanticModel>,
    /// Stores all class symbols in a workspace.
    pub class_defs: HashMap<ClassId, ClassGlobalSymbol>,
    /// Stores Method Global Symbols per Class Global Symbol
    pub method_defs: HashMap<MethodRef, MethodGlobalSymbol>,
    /// Stores Variable Global Symbols per Class Global Symbol
    pub variable_defs: HashMap<MethodRef, HashMap<ScopeId, Vec<VariableGlobalSymbol>>>,
    next_class_id: usize,
}

impl GlobalSemanticModel {
    /// Creates an empty `GlobalSemanticModel` with all tables initialized.
    ///
    /// This initializes storage for classes, methods, variables, local semantic models, and
    /// symbol-definition maps, but does not populate any semantic data.
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            classes: HashMap::new(),
            methods: HashMap::new(),
            lsms: HashMap::new(),
            class_defs: HashMap::new(),
            method_defs: HashMap::new(),
            variable_defs: HashMap::new(),
            next_class_id: 0,
        }
    }

    /// Given a Variable, adds the variable to the vec corresponding to the class and method the variable is defined in.
    /// Returns PublicVarId, which corresponds to the index which the Variable is stored.
    pub fn new_variable(
        &mut self,
        variable: Variable,
        method_ref: MethodRef,
        scope_id: ScopeId,
    ) -> VariableRef {
        let scopes_to_vars = self.variables.entry(method_ref).or_insert(HashMap::new());
        let vars = scopes_to_vars.entry(scope_id).or_insert(Vec::new());
        let var_ref = VariableRef {
            pub_id: Some(PublicVarId(vars.len())),
            priv_id: None,
        };
        vars.push(variable);
        var_ref
    }

    /// Given a Class, adds the class to the `self.classes` vec, returning ClassId, which
    /// corresponds to the index that the Class is stored.
    pub fn new_class(&mut self, class: Class, class_id: ClassId) {
        self.classes.insert(class_id, class);
    }

    /// Given a Method, adds the method to the vec corresponding to the class the method is defined in.
    pub fn new_method(&mut self, method: Method, method_ref: MethodRef) {
        self.methods.insert(method_ref, method);
    }

    /// Inserts a new `LocalSemanticModel`, hashed by class name, to the global store `self.lsms`
    pub fn new_local_semantic(&mut self, class_id: ClassId, local_semantic: LocalSemanticModel) {
        self.lsms.insert(class_id, local_semantic);
    }

    /// Returns a mutable reference to the local semantic model with the given id.
    ///
    /// Logs a warning and returns `None` if `lsm_id` is out of bounds.
    pub fn get_local_semantic_mut(
        &mut self,
        class_id: &ClassId,
    ) -> Option<&mut LocalSemanticModel> {
        self.lsms.get_mut(class_id)
    }

    /// Returns an immutable reference to the local semantic model with the given id.
    ///
    /// Logs a warning and returns `None` if `lsm_id` is out of bounds.
    pub fn get_local_semantic(&self, class_id: &ClassId) -> Option<&LocalSemanticModel> {
        self.lsms.get(class_id)
    }

    /// Returns an immutable reference to the class at `index` in the classes table.
    ///
    /// Logs a warning and returns `None` if `index` is out of bounds.
    pub fn get_class(&self, index: &ClassId) -> Option<&Class> {
        self.classes.get(index)
    }

    /// Returns the `ClassGlobalSymbol` at `index` in the class symbol table.
    ///
    /// Logs a warning and returns `None` if `index` is out of bounds.
    pub fn get_class_symbol(&self, index: &ClassId) -> Option<&ClassGlobalSymbol> {
        self.class_defs.get(index)
    }

    /// Fetches a mutable reference to a method by `MethodRef`.
    ///
    /// Looks up the corresponding method for `method_ref` and then indexes into it. Logs and returns `None`
    /// if the class has no recorded method for `MethodRef`.
    pub fn get_mut_method(&mut self, method_ref: &MethodRef) -> Option<&mut Method> {
        self.methods.get_mut(method_ref)
    }

    /// Fetches an immutable reference to a method by `MethodRef`.
    ///
    /// Looks up the corresponding method for `method_ref` and then indexes into it. Logs and returns `None`
    /// if the class has no recorded method for `MethodRef`.
    pub fn get_method(&self, method_ref: &MethodRef) -> Option<&Method> {
        self.methods.get(method_ref)
    }

    /// Returns the `MethodGlobalSymbol` for a class symbol by symbol index.
    ///
    /// Logs and returns `None` if the class has no method symbols recorded or `method_symbol_id` is
    /// out of bounds.
    pub fn get_method_symbol_mut(
        &mut self,
        method_symbol_ref: &MethodRef,
    ) -> Option<&mut MethodGlobalSymbol> {
        self.method_defs.get_mut(method_symbol_ref)
    }

    /// Returns the `MethodGlobalSymbol` for a class symbol by symbol index.
    ///
    /// Logs and returns `None` if the class has no method symbols recorded or `method_symbol_id` is
    /// out of bounds.
    pub fn get_method_symbol(&self, method_symbol_ref: &MethodRef) -> Option<&MethodGlobalSymbol> {
        self.method_defs.get(method_symbol_ref)
    }

    /// Returns the `VariableGlobalSymbol` for a class symbol by symbol index.
    ///
    /// Logs and returns `None` if the class has no variable symbols recorded or `index` is out of bounds.
    pub fn get_variable_symbol(
        &self,
        method_symbol_ref: &MethodRef,
        index: usize,
        scope_id: &ScopeId,
    ) -> Option<&VariableGlobalSymbol> {
        if let Some(scopes_to_var_symbols) = self.variable_defs.get(method_symbol_ref)
            && let Some(var_symbols) = scopes_to_var_symbols.get(scope_id)
        {
            return var_symbols.get(index);
        }
        None
    }

    /// Returns the `VariableGlobalSymbol` for a MethodRef by symbol index.
    ///
    /// Logs and returns `None` if the class has no variable symbols recorded or `index` is out of bounds.
    pub fn get_variable(
        &self,
        method_ref: &MethodRef,
        index: usize,
        scope_id: &ScopeId,
    ) -> Option<&Variable> {
        if let Some(scopes_to_vars) = self.variables.get(method_ref)
            && let Some(variables) = scopes_to_vars.get(scope_id)
        {
            return variables.get(index);
        }
        None
    }

    /// Clears all semantic state associated with a re-parsed document.
    ///
    /// Resets the class entry, removes method/variable tables for `class_id`, and clears the
    /// associated local semantic model. Use this when a document is being reparsed, not deleted.
    pub fn incremental_reset_doc_semantics(
        &mut self,
        class_id: &ClassId,
        class_name: String,
        methods_to_remove: Vec<MethodRef>,
    ) {
        let Some(class) = self.classes.get_mut(class_id) else {
            eprintln!("Error: class named {:?} not found", class_name);
            return;
        };

        let mut method_names_to_remove = Vec::new();
        for method_ref in &methods_to_remove {
            if let Some(method) = self.methods.remove(method_ref) {
                method_names_to_remove.push(method.name.clone());
            }
            self.variables.remove(&method_ref);
        }
        class.partial_clear(class_name.clone(), true, method_names_to_remove);

        // reset everything in the local semantic model
        if let Some(local_semantic_model) = self.lsms.get_mut(class_id) {
            local_semantic_model.partial_clear(methods_to_remove);
        };
    }

    /// Clears all semantic state associated with a re-parsed document.
    ///
    /// Resets the class entry, removes method/variable tables for `class_id`, and clears the
    /// associated local semantic model. Use this when a document is being reparsed, not deleted.
    pub fn reset_doc_semantics(&mut self, class_id: &ClassId, class_name: String) {
        let Some(class) = self.classes.get_mut(class_id) else {
            eprintln!("Error: class named {:?} not found", class_name);
            return;
        };

        for method_ref in class.methods.values() {
            self.methods.remove(&method_ref);
            self.variables.remove(&method_ref);
        }
        class.clear(class_name.clone(), true);

        // reset everything in the local semantic model
        if let Some(local_semantic_model) = self.lsms.get_mut(&class_id) {
            local_semantic_model.clear();
        };
    }

    pub fn next_id(&mut self) -> usize {
        let id = self.next_class_id;
        self.next_class_id += 1;
        id
    }

    /// Marks the class symbol as inactive and removes all method/variable symbols for the document.
    pub fn remove_document_symbols(
        &mut self,
        class_symbol_id: &ClassId,
        method_symbol_refs_to_remove: &Vec<MethodRef>,
    ) {
        let Some(class_symbol) = self.class_defs.get_mut(class_symbol_id) else {
            eprintln!("Error: in remove_document_symbols, Error: class symbol not found");
            return;
        };
        for method_symbol_ref in method_symbol_refs_to_remove {
            self.method_defs.remove(method_symbol_ref);
            self.variable_defs.remove(method_symbol_ref);
        }
        class_symbol.alive = false;
    }

    /// Updates an existing class symbol’s metadata and marks it as alive.
    pub fn update_class_symbol(
        &mut self,
        name: String,
        range: Range,
        url: Url,
        symbol_id: &ClassId,
    ) {
        let Some(symbol) = self.class_defs.get_mut(symbol_id) else {
            eprintln!("Error: (In global_semantic, update_class_symbol) class symbol not found");
            return;
        };
        symbol.alive = true;
        symbol.name = name;
        symbol.location = range;
        symbol.url = url;
    }

    /// Creates a new class symbol entry and returns its id.
    pub fn new_class_symbol(&mut self, name: String, range: Range, url: Url, symbol_id: ClassId) {
        self.class_defs.insert(
            symbol_id,
            ClassGlobalSymbol {
                name,
                url,
                location: range,
                alive: true,
            },
        );
    }

    /// Adds a new method symbol under `class_symbol_id` and returns its per-class symbol id.
    ///
    /// Returns `None` (and logs) if the per-class method symbol table cannot be retrieved.
    pub fn new_method_symbol(
        &mut self,
        name: String,
        range: Range,
        url: Url,
        method_symbol_ref: MethodRef,
    ) {
        let method_symbol = MethodGlobalSymbol {
            name,
            url,
            location: range,
            references: Vec::new(),
            method_dependencies: Vec::new(),
        };
        self.method_defs.insert(method_symbol_ref, method_symbol);
    }

    /// Adds a new variable symbol under `class_symbol_id` and returns its per-class symbol id.
    ///
    /// Returns `None` (and logs) if the per-class variable symbol table cannot be retrieved.
    pub fn new_variable_symbol(
        &mut self,
        range: Range,
        url: Url,
        var_dependencies: Vec<String>,
        method_symbol_ref: MethodRef,
        variable_ref: VariableRef,
        scope_id: ScopeId,
    ) {
        let scopes_to_vars = self
            .variable_defs
            .entry(method_symbol_ref)
            .or_insert(HashMap::new());
        let defs = scopes_to_vars.entry(scope_id).or_insert(Vec::new());
        if let Some(id) = variable_ref.pub_id {
            if defs.len() != id.0 {
                eprintln!(
                    "ERROR: The index for the variables vec is not equivalent to the index for the variable symbol"
                );
            }
            defs.push(VariableGlobalSymbol {
                url,
                location: range,
                var_dependencies,
            });
        }
    }

    /// Computes effective class keyword values (procedure block + default language) from inheritance.
    ///
    /// Fills only missing (`None`) values using the primary parent (leftmost) transitively, with
    /// cycle protection via DFS state/memoization.
    pub fn class_keyword_inheritance(&mut self) {
        #[derive(Clone)]
        struct Snap {
            declared_pb: Option<bool>,
            declared_lang: Option<Language>,
            primary_parent: Option<ClassId>, // leftmost only
        }

        let mut entries: Vec<(ClassId, &Class)> =
            self.classes.iter().map(|(&id, c)| (id, c)).collect();
        entries.sort_by_key(|(id, _)| id.0);

        let id_to_idx: HashMap<ClassId, usize> = entries
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();

        let class_ids: Vec<ClassId> = entries.iter().map(|(id, _)| *id).collect();

        let snaps: Vec<Snap> = entries
            .iter()
            .map(|(_, c)| Snap {
                declared_pb: c.is_procedure_block,
                declared_lang: c.default_language.clone(),
                primary_parent: c.inherited_classes.get(0).copied(),
            })
            .collect();

        let n = snaps.len();
        let mut memo: Vec<Option<(Option<bool>, Option<Language>)>> = vec![None; n];
        let mut state: Vec<DfsState> = vec![DfsState::Unvisited; n];

        fn dfs(
            idx: usize,
            snaps: &Vec<Snap>,
            id_to_idx: &HashMap<ClassId, usize>,
            memo: &mut Vec<Option<(Option<bool>, Option<Language>)>>,
            state: &mut Vec<DfsState>,
        ) -> (Option<bool>, Option<Language>) {
            if let Some(v) = memo[idx].clone() {
                return v;
            }

            if state[idx] == DfsState::Visiting {
                let s = &snaps[idx];
                return (s.declared_pb, s.declared_lang.clone());
            }

            state[idx] = DfsState::Visiting;

            let s = &snaps[idx];

            // start with declared values
            let mut pb = s.declared_pb;
            let mut lang = s.declared_lang.clone();

            // fill missing from primary parent transitively
            if pb.is_none() || lang.is_none() {
                if let Some(parent) = s.primary_parent {
                    if let Some(&parent_idx) = id_to_idx.get(&parent) {
                        let (ppb, plang) = dfs(parent_idx, snaps, id_to_idx, memo, state);
                        if pb.is_none() {
                            pb = ppb;
                        }
                        if lang.is_none() {
                            lang = plang;
                        }
                    }
                }
            }

            state[idx] = DfsState::Done;
            memo[idx] = Some((pb, lang.clone()));
            (pb, lang)
        }

        // ---- Phase B: apply (only fill None) ----
        for i in 0..n {
            let (eff_pb, eff_lang) = dfs(i, &snaps, &id_to_idx, &mut memo, &mut state);
            let class_id = class_ids[i];
            let Some(cls) = self.classes.get_mut(&class_id) else {
                continue;
            };

            if cls.is_procedure_block.is_none() {
                cls.is_procedure_block = eff_pb;
            }
            if cls.default_language.is_none() {
                cls.default_language = eff_lang;
            }
        }
    }

    /// Build a reverse inheritance index for each class.
    ///
    /// For every class, this returns the transitive set of subclasses that depend on it via
    /// `Extends`. Inactive classes are skipped.
    pub fn build_dependents(&self) -> Dependents {
        let mut entries: Vec<(ClassId, &Class)> =
            self.classes.iter().map(|(&id, c)| (id, c)).collect();
        entries.sort_by_key(|(id, _)| id.0);

        let id_to_idx: HashMap<ClassId, usize> = entries
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();

        let n = entries.len();
        let mut children: Vec<Vec<ClassId>> = vec![Vec::new(); n];

        for (child_id, cls) in entries.iter() {
            if !cls.active {
                continue;
            }
            for parent_id in &cls.inherited_classes {
                if let Some(&parent_idx) = id_to_idx.get(parent_id) {
                    if entries[parent_idx].1.active {
                        children[parent_idx].push(*child_id);
                    }
                }
            }
        }

        let mut memo: Vec<Option<HashSet<ClassId>>> = vec![None; n];
        let mut state: Vec<DfsState> = vec![DfsState::Unvisited; n];
        let mut index = Dependents::new();

        fn dfs(
            idx: usize,
            children: &Vec<Vec<ClassId>>,
            id_to_idx: &HashMap<ClassId, usize>,
            memo: &mut Vec<Option<HashSet<ClassId>>>,
            state: &mut Vec<DfsState>,
        ) -> HashSet<ClassId> {
            if let Some(cached) = &memo[idx] {
                return cached.clone();
            }

            if state[idx] == DfsState::Visiting {
                eprintln!("Cycle detected in inheritance graph");
                generic_exit_statements("GlobalSemanticModel", "build_dependents");
                return HashSet::new();
            }

            state[idx] = DfsState::Visiting;

            let mut table: HashSet<ClassId> = HashSet::new();
            for &child in &children[idx] {
                table.insert(child);
                if let Some(&child_idx) = id_to_idx.get(&child) {
                    table.extend(dfs(child_idx, children, id_to_idx, memo, state));
                }
            }

            state[idx] = DfsState::Done;
            memo[idx] = Some(table.clone());
            table
        }

        for (i, (class_id, cls)) in entries.iter().enumerate() {
            if !cls.active {
                continue;
            }
            let mut dependents: Vec<ClassId> = dfs(i, &children, &id_to_idx, &mut memo, &mut state)
                .into_iter()
                .collect();
            dependents.sort_by_key(|class_id| class_id.0);
            index.dependent_classes.insert(*class_id, dependents);
        }

        index
    }

    /// Builds an override/dispatch index for methods across the inheritance graph.
    ///
    /// Produces:
    /// - per-class effective public method table,
    /// - override relationships (`overrides` / `overridden_by`) for public and private declarations.
    ///
    /// IMPORTANT: `class.inherited_classes` must contain direct parents only when called.
    pub fn build_override_index(&self) -> OverrideIndex {
        #[derive(Clone)]
        struct ClassSnap {
            class_id: ClassId,
            parents: Vec<ClassId>,
            inheritance_direction: String, // "left" or "right"
            public_methods: Vec<(String, MethodRef)>, // declared public methods in this class
            private_methods: Vec<(String, MethodRef)>,
        }

        let mut entries: Vec<(ClassId, &Class)> =
            self.classes.iter().map(|(&id, c)| (id, c)).collect();
        entries.sort_by_key(|(id, _)| id.0);

        let id_to_idx: HashMap<ClassId, usize> = entries
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();

        let snaps: Vec<ClassSnap> = entries
            .iter()
            .map(|(id, c)| ClassSnap {
                class_id: *id,
                parents: c.inherited_classes.clone(),
                inheritance_direction: c.inheritance_direction.clone(),
                public_methods: c
                    .methods
                    .iter()
                    .filter(|&(_, method_ref)| self.get_method(method_ref).is_some())
                    .map(|(name, method_ref)| (name.clone(), method_ref.clone()))
                    .collect(),
                private_methods: c
                    .methods
                    .iter()
                    .filter(|&(_, method_ref)| self.get_method(method_ref).is_none())
                    .map(|(name, method_ref)| (name.clone(), method_ref.clone()))
                    .collect(),
            })
            .collect();

        let n = snaps.len();
        let mut memo: Vec<Option<HashMap<String, (MethodRef, bool)>>> = vec![None; n];
        let mut state: Vec<DfsState> = vec![DfsState::Unvisited; n];
        let mut index = OverrideIndex::new();

        fn dfs(
            idx: usize,
            snaps: &Vec<ClassSnap>,
            id_to_idx: &HashMap<ClassId, usize>,
            memo: &mut Vec<Option<HashMap<String, (MethodRef, bool)>>>,
            state: &mut Vec<DfsState>,
            index: &mut OverrideIndex,
        ) -> HashMap<String, (MethodRef, bool)> {
            if let Some(cached) = memo[idx].clone() {
                return cached;
            }
            if state[idx] == DfsState::Visiting {
                eprintln!("Cycle detected in inheritance graph");
                generic_exit_statements("GlobalSemanticModel", "build_override_index");
                return HashMap::new();
            }

            state[idx] = DfsState::Visiting;

            let snap = &snaps[idx];
            let cls_id = snap.class_id;

            // inherited effective table
            let mut table: HashMap<String, (MethodRef, bool)> = HashMap::new();

            let parent_iter: Box<dyn Iterator<Item = &ClassId>> =
                if snap.inheritance_direction == "right" {
                    Box::new(snap.parents.iter().rev())
                } else {
                    Box::new(snap.parents.iter())
                };

            for parent in parent_iter {
                let Some(&parent_idx) = id_to_idx.get(parent) else {
                    continue;
                };
                let parent_table = dfs(parent_idx, snaps, id_to_idx, memo, state, index);
                for (name, mref) in parent_table {
                    table.entry(name).or_insert(mref); // first wins
                }
            }

            // overlay declared methods for this class
            for (name, child_ref) in &snap.public_methods {
                if let Some((base_ref, is_public)) = table.get(name).copied()
                    && is_public
                {
                    index.overrides.insert(child_ref.clone(), base_ref);
                    index
                        .overridden_by
                        .entry(base_ref)
                        .or_default()
                        .push(child_ref.clone());
                }
                table.insert(name.clone(), (*child_ref, true)); // child wins
            }

            for (name, child_ref) in &snap.private_methods {
                if let Some((base_ref, _)) = table.get(name).copied() {
                    index.overrides.insert(child_ref.clone(), base_ref);
                    index
                        .overridden_by
                        .entry(base_ref)
                        .or_default()
                        .push(child_ref.clone());
                }

                table.insert(name.clone(), (*child_ref, false)); // child wins
            }
            let effective_public: HashMap<String, MethodRef> = table
                .iter()
                .filter(|&(_, (_, is_public))| *is_public)
                .map(|(name, (method_ref, _))| (name.clone(), method_ref.clone()))
                .collect();

            index
                .effective_public_methods
                .insert(cls_id, effective_public);

            state[idx] = DfsState::Done;
            memo[idx] = Some(table.clone());
            table
        }

        for i in 0..n {
            let _ = dfs(i, &snaps, &id_to_idx, &mut memo, &mut state, &mut index);
        }
        index
    }
}
