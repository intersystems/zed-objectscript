use crate::common::{generic_exit_statements, point_in_range};
use crate::parse_structures::{ClassId, MethodId, MethodRef, VariableRef};
use crate::scope_structures::*;
use std::collections::HashMap;
use tree_sitter::{Point, Range};

/// A lexical scope within a document.
#[derive(Clone, Debug)]
pub struct Scope {
    /// Start Point of Scope.
    pub start: Point,
    /// End Point of Scope.
    pub end: Point,
    /// Optional: Id of Parent Scope.
    pub parent: Option<ScopeId>,
    /// Ids of Child Scopes.
    pub children: Vec<ScopeId>,
    /// Stores the Variable Symbols defined in this scope.
    pub variable_symbols: Vec<VariableSymbol>,
    /// Stores variable name -> VariableRef for public variables defined in this scope.
    pub public_var_defs: HashMap<String, Vec<VariableRef>>,
    /// Stores variable name -> VariableRef for private variables defined in this scope.
    pub private_variable_defs: HashMap<String, Vec<VariableRef>>,
    /// Optional: Name of method that this scope is a part of
    pub method: Option<String>,
}
impl Scope {
    /// Create a new scope node with the given bounds and optional parent.
    fn new(start: Point, end: Point, parent: Option<ScopeId>, method_name: Option<String>) -> Self {
        Self {
            start,
            end,
            parent,
            children: Vec::new(),
            variable_symbols: Vec::new(),
            public_var_defs: HashMap::new(), // HashMap var name -> GlobalSymbol
            private_variable_defs: HashMap::new(),
            method: method_name,
        }
    }

    /// Returns a reference to the variable symbol at the given index.
    pub fn get_variable_symbol(&self, index: usize) -> Option<&VariableSymbol> {
        self.variable_symbols.get(index)
    }

    /// Look up a private variable definition in this scope by name and return its source range.
    ///
    /// Logs a warning and returns `None` if the name is not present or the stored symbol id is
    /// out of bounds for `variable_symbols`.
    pub fn get_variable_location(&self, variable_name: &str) -> Vec<Range> {
        let mut variable_locations = Vec::new();
        if let Some(variable_refs) = self.private_variable_defs.get(variable_name) {
            for variable_ref in variable_refs {
                if variable_ref.pub_id.is_some() {
                    return variable_locations;
                }
                if let Some(variable_id) = variable_ref.priv_id
                    && let Some(variable_symbol) = self.variable_symbols.get(variable_id.0)
                {
                    variable_locations.push(variable_symbol.location);
                }
            }
        }
        variable_locations
    }

    /// Define a new private variable symbol in this scope and return its `VariableRef`.
    pub fn new_variable_symbol(
        &mut self,
        name: String,
        location: Range,
        var_dependencies: Vec<String>,
        var_ref: VariableRef,
    ) {
        if let Some(i) = var_ref.priv_id {
            if i.0 != self.variable_symbols.len() {
                eprintln!(
                    "ERROR: Variable Index is NOT equivalent to the variable symbols index (new_variable_symbol, scope tree)"
                );
            }
            self.private_variable_defs
                .entry(name)
                .or_insert_with(Vec::new)
                .push(var_ref);
            self.variable_symbols.push(VariableSymbol {
                location,
                references: Vec::new(),
                var_dependencies,
            });
        }
    }

    /// Record a public variable definition in this scope by mapping its name to a global symbol id.
    pub fn new_symbol_pub_variable(&mut self, name: String, variable_reference: VariableRef) {
        self.public_var_defs
            .entry(name)
            .or_insert_with(Vec::new)
            .push(variable_reference);
    }

    /// Look up if `variable_name` is defined in this scope, and return Vec<VariableRef>
    /// that contains all the definitions.
    ///
    /// returns `None` if the name is not present.
    pub fn get_variable_references(&self, variable_name: &str) -> Vec<VariableRef> {
        if let Some(locs) = self.private_variable_defs.get(variable_name) {
            return locs.clone();
        } else if let Some(var_references) = self.public_var_defs.get(variable_name).cloned() {
            return var_references;
        }
        Vec::new()
    }

    /// Look up a public variable definition in this scope by name.
    ///
    /// returns `None` if the name is not present.
    pub fn get_pub_variable_symbol(&self, name: &str) -> Vec<VariableRef> {
        if let Some(var_references) = self.public_var_defs.get(name).cloned() {
            return var_references;
        }
        Vec::new()
    }
}

/// Per-document scope index used for symbol lookup and resolution.
#[derive(Debug)]
pub struct ScopeTree {
    /// Stores ScopeId -> Scope for all Scopes in the document.
    pub scopes: HashMap<ScopeId, Scope>,
    /// The root ScopeId, which spans the whole document.
    pub root: ScopeId,
    /// The iterator that keeps track of the Id to assign to the next scope.
    pub next_scope_id: usize,
    /// Stores methodId -> Method Symbol for all private methods in the document.
    pub private_method_defs: HashMap<MethodId, MethodSymbol>,
    /// The Id corresponding to the class definition symbol for this document when this is a class file.
    pub class_def: Option<ClassId>,
}

impl Clone for ScopeTree {
    /// Clone the ScopeTree
    fn clone(&self) -> Self {
        Self {
            scopes: self.scopes.clone(),
            root: self.root,
            next_scope_id: self.next_scope_id,
            private_method_defs: self.private_method_defs.clone(),
            class_def: self.class_def,
        }
    }
}

impl ScopeTree {
    /// Creates a new scope tree with a root scope spanning the entire document.
    pub fn new(class_def: Option<ClassId>) -> Self {
        let root_id = ScopeId(0);
        let root_scope = Scope::new(
            Point { row: 0, column: 0 },
            Point {
                row: usize::MAX,
                column: usize::MAX,
            },
            None,
            None,
        );
        let mut scopes = HashMap::new();
        scopes.insert(root_id, root_scope);
        Self {
            scopes,
            root: root_id,
            next_scope_id: 1,
            private_method_defs: HashMap::new(),
            class_def,
        }
    }

    /// Look up a private method symbol by name.
    ///
    /// Logs a warning and returns `None` if it does not exist.
    pub fn get_private_method_symbol(&self, method_id: &MethodId) -> Option<&MethodSymbol> {
        self.private_method_defs.get(method_id)
    }

    /// Add a new child scope to `parent`, returning the new `ScopeId`.
    pub fn add_scope(
        &mut self,
        start: Point,
        end: Point,
        parent: ScopeId,
        method_name: Option<String>,
    ) -> ScopeId {
        let scope_id = ScopeId(self.next_scope_id);
        self.next_scope_id += 1;
        let scope = Scope::new(start, end, Some(parent), method_name);
        // update parent to include this scope as a child
        if let Some(parent_scope) = self.scopes.get_mut(&parent) {
            parent_scope.children.push(scope_id);
        }
        self.scopes.insert(scope_id, scope);
        scope_id
    }

    /// Inserts a private variable symbol into the scope containing its start point.
    pub fn new_variable_symbol(
        &mut self,
        name: String,
        range: Range,
        var_deps: Vec<String>,
        variable_reference: VariableRef,
    ) {
        let Some(scope) = self.get_mut_scope(range.start_point) else {
            eprintln!("Error: couldn't get scope for variable {:?}", name);
            return;
        };
        scope.new_variable_symbol(name, range, var_deps, variable_reference)
    }

    /// Register a private method definition symbol in this document.
    pub fn new_method_symbol(&mut self, name: String, range: Range, method_ref: MethodRef) {
        let method_symbol = MethodSymbol {
            name: name.clone(),
            location: range,
            references: Vec::new(),
            method_dependencies: Vec::new(),
            method_ref,
        };
        self.private_method_defs
            .insert(method_ref.id, method_symbol);
    }

    /// Get a mutable reference to the innermost scope containing `point`.
    ///
    /// Logs a warning and returns `None` if no containing scope is found.
    fn get_mut_scope(&mut self, point: Point) -> Option<&mut Scope> {
        let Some(scope_id) = self.find_current_scope(point) else {
            eprintln!("Warning: Scope Id not found for Point {:?}", point);
            return None;
        };

        let scopes = self.scopes.clone();
        let Some(scope) = self.scopes.get_mut(&scope_id) else {
            eprintln!(
                "Warning: Scope not found, Scope Id {:?} DNE in scopes hashmap: \n {:?} \n\n",
                scope_id, scopes
            );
            return None;
        };
        Some(scope)
    }

    /// Get an immutable reference to the innermost scope containing `point`.
    ///
    /// Logs a warning and returns `None` if no containing scope is found.
    pub fn get_scope(&self, point: Point) -> Option<&Scope> {
        let Some(scope_id) = self.find_current_scope(point) else {
            eprintln!("Warning: Scope Id not found for Point {:?}", point);
            return None;
        };
        let Some(scope) = self.scopes.get(&scope_id) else {
            eprintln!(
                "Warning: Scope not found, Scope Id {:?} DNE in scopes hashmap: \n {:?} \n\n",
                scope_id, self.scopes
            );
            return None;
        };

        Some(scope)
    }
    pub fn get_scope_children(&self, scope_id: &ScopeId) -> Vec<ScopeId> {
        let Some(scope) = self.scopes.get(&scope_id) else {
            return Vec::new();
        };
        let mut children = Vec::new();
        for child_id in &scope.children {
            children.push(*child_id);
            children.extend(self.get_scope_children(child_id))
        }
        children
    }

    /// Record a public variable symbol in the scope that contains `range.start_point`.
    pub fn new_public_var_symbol(
        &mut self,
        name: String,
        range: Range,
        variable_reference: VariableRef,
    ) {
        let Some(scope) = self.get_mut_scope(range.start_point) else {
            generic_exit_statements("Scope", "new_public_var_symbol");
            return;
        };
        scope.new_symbol_pub_variable(name.clone(), variable_reference);
    }

    /// Returns the method name associated with the scope containing the given position.
    pub fn get_method_name(&self, pos: Point) -> Option<String> {
        let mut curr_scope = if let Some(scope) = self.get_scope(pos) {
            scope
        } else {
            return None;
        };
        while curr_scope.method.is_none()
            && let Some(parent_id) = curr_scope.parent
            && let Some(parent) = self.scopes.get(&parent_id)
        {
            curr_scope = parent
        }
        curr_scope.method.clone()
    }

    /// Returns a reference to a variable symbol in the scope at the given position.
    pub fn get_variable_symbol(
        &self,
        variable_index: usize,
        scope_id: &ScopeId,
    ) -> Option<&VariableSymbol> {
        let Some(scope) = self.scopes.get(scope_id) else {
            generic_exit_statements("Scope", "get_variable_symbol");
            return None;
        };

        scope.get_variable_symbol(variable_index)
    }

    pub fn get_oref_references(
        &self,
        variable_name: &str,
        scope_id: ScopeId,
    ) -> Vec<(ScopeId, Vec<VariableRef>)> {
        let mut var_defs = Vec::new();
        let Some(scope) = self.scopes.get(&scope_id) else {
            return Vec::new();
        };

        let refs = scope.get_variable_references(variable_name);
        if !refs.is_empty() {
            var_defs.push((scope_id, refs));
        }
        for child_scope_id in &scope.children {
            var_defs.extend(self.get_oref_references(variable_name, *child_scope_id));
        }
        var_defs
    }

    /// Look up a private variable definition.
    pub fn get_variable_definition(
        &self,
        variable_name: &str,
        scope_id: ScopeId,
    ) -> Vec<(ScopeId, Vec<Range>)> {
        let mut var_defs = Vec::new();
        let Some(scope) = self.scopes.get(&scope_id) else {
            return Vec::new();
        };
        let scopes_var_defs = scope.get_variable_location(variable_name);
        if !scopes_var_defs.is_empty() {
            var_defs.push((scope_id, scopes_var_defs));
        }
        for child_scope_id in &scope.children {
            var_defs.extend(self.get_variable_definition(variable_name, *child_scope_id));
        }
        var_defs
    }

    /// Find all variable refs for public variable `var_name`
    pub fn pub_variable_in_scope(
        &self,
        var_name: &str,
        scope_id: ScopeId,
    ) -> Vec<(ScopeId, Vec<VariableRef>)> {
        let mut var_defs = Vec::new();
        let Some(scope) = self.scopes.get(&scope_id) else {
            return Vec::new();
        };

        let refs = scope.get_pub_variable_symbol(var_name);
        if !refs.is_empty() {
            var_defs.push((scope_id, refs));
        }
        for child_scope_id in &scope.children {
            var_defs.extend(self.pub_variable_in_scope(var_name, *child_scope_id));
        }
        var_defs
    }

    /// Find the innermost scope containing `pos` by descending from the root into matching children.
    pub fn find_current_scope(&self, pos: Point) -> Option<ScopeId> {
        let mut current = self.root;

        loop {
            let Some(scope) = self.scopes.get(&current) else {
                return None;
            };
            // iterate over children vector (which contains scopeid values)
            // searches for the first child that satisfies the condition of containing the point
            let child = scope.children.iter().find(|&&child_id| {
                let Some(child_scope) = self.scopes.get(&child_id) else {
                    return false;
                };
                point_in_range(pos, child_scope.start, child_scope.end)
            });
            match child {
                Some(&child_id) => current = child_id,
                None => {
                    return Some(current);
                }
            }
        }
    }
}
