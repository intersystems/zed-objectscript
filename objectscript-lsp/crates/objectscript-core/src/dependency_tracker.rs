use crate::parse_structures::ClassId;
use crate::parse_structures::MethodRef;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::{Dfs, Reversed};
use std::collections::HashMap;

/// Stores all subclasses that depend on a given class through inheritance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dependents {
    pub dependent_classes: HashMap<ClassId, Vec<ClassId>>,
}

impl Dependents {
    /// Creates an empty Dependents index.
    pub fn new() -> Self {
        Self {
            dependent_classes: HashMap::new(),
        }
    }
}

/// Directed graph of method call relationships (node = MethodRef, edge = caller->callee).
#[derive(Clone, Debug)]
pub struct DependencyGraph {
    pub graph: DiGraph<MethodRef, ()>,
    pub lookup: HashMap<MethodRef, NodeIndex>,
}

impl DependencyGraph {
    /// Creates an empty dependency graph.
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            lookup: HashMap::new(),
        }
    }

    /// Returns the graph NodeIndex for a method, if it exists.
    pub fn get_node(&self, method: MethodRef) -> Option<&NodeIndex> {
        self.lookup.get(&method)
    }
    /// If node already exists, index is returned. Otherwise, node is added
    /// to the graph.
    pub fn get_or_add_node(&mut self, method: MethodRef) -> NodeIndex {
        if let Some(&idx) = self.lookup.get(&method) {
            return idx;
        }

        let idx = self.graph.add_node(method);
        self.lookup.insert(method, idx);
        idx
    }

    /// Returns all transitive callers of a method via reverse DFS.
    pub fn all_ancestors(&self, target: NodeIndex) -> Vec<MethodRef> {
        let reversed = Reversed(&self.graph);
        let mut dfs = Dfs::new(&reversed, target);
        let mut ancestors = Vec::new();
        while let Some(node) = dfs.next(&reversed) {
            if node != target {
                ancestors.push(self.graph[node]);
            }
        }
        ancestors
    }

    /// Adds a caller->callee edge, creating nodes if needed.
    pub fn add_edge(&mut self, caller: MethodRef, callee: MethodRef) {
        let caller_idx = self.get_or_add_node(caller);
        let callee_idx = self.get_or_add_node(callee);

        // caller -> callee
        self.graph.update_edge(caller_idx, callee_idx, ());
    }
}
