use crate::parse_structures::{MethodRef, ParameterRef, PropertyRef};
use std::collections::HashMap;

/// Stores information about what superclass methods get overwritten, and by which subclass.
/// Stores the methods available for each class.
/// For completion / resolution, this must be built after inheritance + overrides
#[derive(Default, Debug, Clone)]
pub struct OverrideIndex {
    /// Stores the MethodRef that a class sees for each method name (keyed by class name)
    pub effective_methods: HashMap<String, HashMap<String, MethodRef>>,

    /// Stores the PropertyRef that a class sees for each property name (keyed by class name)
    pub effective_properties: HashMap<String, HashMap<String, PropertyRef>>,

    /// Stores the ParameterRef that a class sees for each parameter name (keyed by class name)
    pub effective_parameters: HashMap<String, HashMap<String, ParameterRef>>,

    /// subclass method ref (the method that overwites the superclass one) -> superclass method ref
    pub method_overrides: HashMap<MethodRef, MethodRef>,

    /// superclass method ref -> subclass method refs (subclass methods that overwrote the superclass)
    pub method_overridden_by: HashMap<MethodRef, Vec<MethodRef>>,

    /// subclass property ref (the property that overwites the superclass one) -> superclass property ref
    pub property_overrides: HashMap<PropertyRef, PropertyRef>,

    /// superclass property ref -> subclass property refs (subclass property that overwrote the superclass)
    pub property_overridden_by: HashMap<PropertyRef, Vec<PropertyRef>>,

    /// subclass parameter ref (the parameter that overwites the superclass one) -> superclass parameter ref
    pub parameter_overrides: HashMap<ParameterRef, ParameterRef>,

    /// superclass parameter ref -> subclass ParameterRef (subclass parameter that overwrote the superclass)
    pub parameter_overridden_by: HashMap<ParameterRef, Vec<ParameterRef>>,
}

impl OverrideIndex {
    /// Creates an empty `OverrideIndex` with all maps initialized.
    ///
    /// This index is typically populated after computing inheritance and resolving class member overrides.
    pub fn new() -> Self {
        Self {
            effective_methods: HashMap::new(),
            method_overrides: HashMap::new(),
            method_overridden_by: HashMap::new(),
            property_overrides: HashMap::new(),
            property_overridden_by: HashMap::new(),
            effective_properties: HashMap::new(),
            parameter_overridden_by: HashMap::new(),
            parameter_overrides: HashMap::new(),
            effective_parameters: HashMap::new(),
        }
    }
}
