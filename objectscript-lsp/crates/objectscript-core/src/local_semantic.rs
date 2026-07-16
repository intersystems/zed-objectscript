use crate::parse_structures::{
    ClassProperty, Method, MethodRef, PrivateVarId, Variable, VariableRef,
};
use std::collections::HashMap;
/// Per-document private semantic state (methods, properties, variables).
///
/// This is used for private members that should not be shared across classes globally.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSemanticModel {
    pub methods: HashMap<MethodRef, Method>,
    pub properties: Vec<ClassProperty>,
    pub variables: HashMap<MethodRef, Vec<Variable>>,
    pub active: bool,
}

impl LocalSemanticModel {
    /// Creates a new, empty `LocalSemanticModel` with `active` set to `true`.
    pub fn new() -> Self {
        Self {
            methods: HashMap::new(),
            properties: Vec::new(),
            variables: HashMap::new(),
            active: true,
        }
    }

    /// Removes methods and their variables by MethodRef, leaving the rest intact.
    pub fn partial_clear(&mut self, methods_to_clear: Vec<MethodRef>) {
        for method_ref in methods_to_clear {
            self.methods.remove(&method_ref);
            self.variables.remove(&method_ref);
        }
    }

    /// Returns a reference to a private variable by method ref and index.
    pub fn get_variable(&self, method_ref: &MethodRef, variable_index: usize) -> Option<&Variable> {
        self.variables
            .get(method_ref)
            .and_then(|variables| variables.get(variable_index))
    }

    /// Clears all stored methods/properties/variables and marks the model as inactive.
    pub fn clear(&mut self) {
        self.methods.clear();
        self.properties.clear();
        self.variables.clear();
        self.active = false;
    }

    /// Adds a new private/local variable to this model and returns its `PrivateVarId`.
    ///
    /// The returned id is the index of the variable in the internal `variables` vector.
    pub fn new_variable(&mut self, method_ref: MethodRef, variable: Variable) -> VariableRef {
        let curr_method_vars = self.variables.entry(method_ref).or_insert(Vec::new());
        let var_ref = VariableRef {
            pub_id: None,
            priv_id: Some(PrivateVarId(curr_method_vars.len())),
        };
        curr_method_vars.push(variable);
        var_ref
    }

    /// Adds a new private/local method to this model and returns its `PrivateMethodId`.
    ///
    /// The returned id is the index of the method in the internal `methods` vector.
    pub fn new_method(&mut self, method: Method, method_ref: MethodRef) {
        self.methods.insert(method_ref, method);
    }

    /// Returns an immutable reference to the private/local method at `private_method_id`.
    ///
    /// Logs a warning and returns `None` if the index is out of bounds.
    pub fn get_method(&self, method_ref: &MethodRef) -> Option<&Method> {
        self.methods.get(method_ref)
    }

    /// Returns a mutable reference to the private/local method at `index`.
    ///
    /// Logs a warning and returns `None` if the index is out of bounds.
    pub fn get_method_mut(&mut self, method_ref: &MethodRef) -> Option<&mut Method> {
        self.methods.get_mut(method_ref)
    }
}
