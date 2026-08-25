use crate::parse_structures::ClassId;
use crate::parse_structures::MethodRef;
use petgraph::Direction;
use petgraph::graph::{DiGraph, EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet, VecDeque};
use tower_lsp::lsp_types::Range as LspRange;
use tree_sitter::Range;
/// Stores all subclasses that depend on a given class through inheritance.
#[derive(Clone, Debug)]
pub struct Dependents {
    pub dependent_classes: HashMap<ClassId, HashSet<ClassId>>,
    pub direct_subclasses: HashMap<ClassId, HashMap<ClassId, LspRange>>,
}

impl Dependents {
    /// Creates an empty Dependents index.
    pub fn new() -> Self {
        Self {
            dependent_classes: HashMap::new(),
            direct_subclasses: HashMap::new(),
        }
    }

    pub fn get_direct_subclasses(&self, class_id: &ClassId) -> Option<&HashMap<ClassId, LspRange>> {
        self.direct_subclasses.get(class_id)
    }

    pub fn get_transitive_subclasses(&self, class_id: &ClassId) -> Option<&HashSet<ClassId>> {
        self.dependent_classes.get(class_id)
    }

    // NOTE: direct subclasses must be up to date for this to work correctly.
    pub fn rebuild_transitive_subclasses(&mut self, class_id: ClassId) {
        let mut result = HashSet::new();
        let mut queue: VecDeque<ClassId> = VecDeque::new();

        if let Some(direct) = self.direct_subclasses.get(&class_id) {
            queue.extend(direct.keys().copied());
        }

        while let Some(child) = queue.pop_front() {
            if !result.insert(child) {
                continue;
            }
            if let Some(grandchildren) = self.direct_subclasses.get(&child) {
                queue.extend(grandchildren.keys().copied());
            }
        }

        self.dependent_classes.insert(class_id, result);
    }
}

/// Directed graph of method call relationships (node = MethodRef, edge = caller->callee).
#[derive(Clone, Debug)]
pub struct DependencyGraph {
    pub graph: DiGraph<MethodRef, Range>,
    pub lookup: HashMap<MethodRef, NodeIndex>,
    pub class_nodes: HashMap<ClassId, Vec<NodeIndex>>,
}

impl DependencyGraph {
    /// Creates an empty dependency graph.
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            lookup: HashMap::new(),
            class_nodes: HashMap::new(),
        }
    }

    /// Returns the graph NodeIndexes for all methods from a given Class of `class_id`, if it exists.
    pub fn get_class_nodes(&self, class: &ClassId) -> Vec<NodeIndex> {
        if let Some(indices) = self.class_nodes.get(class) {
            return indices.clone();
        } else {
            return Vec::new();
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
        self.class_nodes
            .entry(method.class)
            .or_insert(Vec::new())
            .push(idx);
        idx
    }

    pub fn get_method_ref_from_node_index(&self, node_index: NodeIndex) -> Option<&MethodRef> {
        self.graph.node_weight(node_index)
    }

    pub fn remove_edge(&mut self, target: EdgeIndex) {
        self.graph.remove_edge(target);
    }

    /// Removes and Returns all direct callers of a method IF the caller is from a different class by checking the direct edges to the node at `NodeIndex`.
    pub fn remove_direct_ancestors(
        &mut self,
        target: NodeIndex,
        classes_with_private_access: &HashSet<ClassId>,
    ) -> HashSet<(MethodRef, Range)> {
        // e.source is the caller (if it has the same classid, it won't be removed)
        let incoming: Vec<(EdgeIndex, MethodRef, Range)> = self
            .graph
            .edges_directed(target, Direction::Incoming)
            .filter(|e| !classes_with_private_access.contains(&self.graph[e.source()].class))
            .map(|e| (e.id(), self.graph[e.source()], *e.weight()))
            .collect();
        let mut method_caller_refs = HashSet::new();
        for (edge_id, method_caller_ref, edge_weight) in incoming {
            self.remove_edge(edge_id);
            method_caller_refs.insert((method_caller_ref, edge_weight));
        }
        method_caller_refs
    }

    pub fn remove_incoming_calls_to_node(
        &mut self,
        target: NodeIndex,
    ) -> HashSet<(MethodRef, Range)> {
        // e.source is the caller (if it has the same classid, it won't be removed)
        let incoming: Vec<(EdgeIndex, MethodRef, Range)> = self
            .graph
            .edges_directed(target, Direction::Incoming)
            .map(|e| (e.id(), self.graph[e.source()], *e.weight()))
            .collect();
        let mut method_caller_refs = HashSet::new();
        for (edge_id, method_caller_ref, edge_weight) in incoming {
            self.remove_edge(edge_id);
            method_caller_refs.insert((method_caller_ref, edge_weight));
        }
        method_caller_refs
    }

    /// Removes node representing `method_ref` from the graph.
    /// DiGraph::remove_node swaps the last node into the removed slot,
    /// so this updates `lookup` and `class_nodes` maps
    pub fn remove_node(&mut self, method_ref: MethodRef) {
        let Some(idx) = self.lookup.remove(&method_ref) else {
            return;
        };

        // What node will be swapped into this index?
        let last_idx = NodeIndex::new(self.graph.node_count() - 1);
        let last_method_ref = if last_idx != idx {
            Some(self.graph[last_idx])
        } else {
            None
        };

        // Remove the node (removes all its edges too)
        self.graph.remove_node(idx);

        // If a different node was swapped in, update its lookup entry
        if let Some(swapped) = last_method_ref {
            self.lookup.insert(swapped, idx);
            // Update class_nodes for the swapped method
            if let Some(nodes) = self.class_nodes.get_mut(&swapped.class) {
                if let Some(pos) = nodes.iter().position(|&n| n == last_idx) {
                    nodes[pos] = idx;
                }
            }
        }

        // Remove from class_nodes
        if let Some(nodes) = self.class_nodes.get_mut(&method_ref.class) {
            nodes.retain(|&n| n != idx);
        }
    }

    /// Returns all transitive callers of a method via BFS (closest ancestors first).
    /// Each entry includes the ancestor's MethodRef, the edge Range (the method call range), and the BFS depth.
    pub fn all_ancestors(&self, target: NodeIndex) -> Vec<(MethodRef, Range, usize)> {
        let mut ancestors = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((target, 0usize));
        visited.insert(target);

        while let Some((node, depth)) = queue.pop_front() {
            for edge in self.graph.edges_directed(node, Direction::Incoming) {
                let parent = edge.source();
                if visited.insert(parent) {
                    ancestors.push((self.graph[parent], *edge.weight(), depth + 1));
                    queue.push_back((parent, depth + 1));
                }
            }
        }
        ancestors
    }

    pub fn is_ancestor(&self, ancestor: MethodRef, descendant: MethodRef) -> bool {
        let Some(&ancestor_idx) = self.get_node(ancestor) else {
            return false;
        };
        let Some(&descendant_idx) = self.get_node(descendant) else {
            return false;
        };

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(descendant_idx);
        visited.insert(descendant_idx);

        while let Some(node) = queue.pop_front() {
            for edge in self.graph.edges_directed(node, Direction::Incoming) {
                let parent = edge.source();
                if parent == ancestor_idx {
                    return true;
                }
                if visited.insert(parent) {
                    queue.push_back(parent);
                }
            }
        }
        false
    }

    /// BFS upward from `target`, stopping at nodes where `is_definer` returns true.
    /// Returns the closest definer on each distinct path — i.e. no definer in the result
    /// is an ancestor of another.
    /// Each result includes the MethodRef of the definer and the call-edge Range that led toward it.
    pub fn closest_definers(
        &self,
        target: NodeIndex,
        is_definer: impl Fn(&MethodRef, &Range) -> bool,
    ) -> Vec<(MethodRef, Range)> {
        let mut results = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(target);

        for edge in self.graph.edges_directed(target, Direction::Incoming) {
            let parent = edge.source();
            if visited.insert(parent) {
                queue.push_back((parent, *edge.weight()));
            }
        }

        while let Some((node, call_range)) = queue.pop_front() {
            let method_ref = self.graph[node];
            if is_definer(&method_ref, &call_range) {
                results.push((method_ref, call_range));
            } else {
                for edge in self.graph.edges_directed(node, Direction::Incoming) {
                    let parent = edge.source();
                    if visited.insert(parent) {
                        queue.push_back((parent, *edge.weight()));
                    }
                }
            }
        }

        results
    }

    /// Adds a caller->callee edge, creating nodes if needed.
    pub fn add_edge(&mut self, caller: MethodRef, callee: MethodRef, method_call_range: Range) {
        let caller_idx = self.get_or_add_node(caller);
        let callee_idx = self.get_or_add_node(callee);

        // caller -> callee
        self.graph
            .update_edge(caller_idx, callee_idx, method_call_range);
    }
}
