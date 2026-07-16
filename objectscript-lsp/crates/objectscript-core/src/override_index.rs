use crate::parse_structures::{ClassId, MethodRef};
use std::collections::HashMap;

/// Stores information about what superclass methods get overwritten, and by which subclass.
/// Stores the public methods available for each class.
/// For completion / resolution, this must be built after inheritance + overrides
#[derive(Default, Debug)]
pub struct OverrideIndex {
    /// Stores the Method Id that a class sees for each public method name
    pub effective_public_methods: HashMap<ClassId, HashMap<String, MethodRef>>,

    /// subclass method ref (the method that overwites the superclass one) -> superclass method ref
    pub overrides: HashMap<MethodRef, MethodRef>,

    /// superclass method ref -> subclass method refs (subclass methods that overwrote the superclass)
    pub overridden_by: HashMap<MethodRef, Vec<MethodRef>>,
}

impl OverrideIndex {
    /// Creates an empty `OverrideIndex` with all maps initialized.
    ///
    /// This index is typically populated after computing inheritance and resolving overrides.
    pub fn new() -> Self {
        Self {
            effective_public_methods: HashMap::new(),
            overrides: HashMap::new(),
            overridden_by: HashMap::new(),
        }
    }

    /// Returns a deep clone of the override index.
    ///
    /// Clones all internal maps (`effective_public_methods`, `overrides`, `overridden_by`).
    /// Note: this duplicates `Clone` behavior; consider deriving `Clone` on `OverrideIndex` instead.
    pub fn clone(&self) -> OverrideIndex {
        Self {
            effective_public_methods: self.effective_public_methods.clone(),
            overrides: self.overrides.clone(),
            overridden_by: self.overridden_by.clone(),
        }
    }
}
