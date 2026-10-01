use crate::common::generic_exit_statements;
use crate::dependency_tracker::Dependents;
use crate::local_semantic::LocalSemanticModel;
use crate::parse_structures::{
    Class, ClassId, DfsState, ForeignKey, ForeignKeyRef, Index, IndexRef, Method, MethodRef,
    Parameter, ParameterRef, Projection, ProjectionRef, Property, PropertyRef, PublicVarId, Query,
    QueryRef, Relationship, RelationshipRef, Storage, StorageRef, Trigger, TriggerRef, Variable,
    VariableDefType, VariableRef, XData, XdataRef,
};
use crate::scope_structures::{
    ClassGlobalSymbol, ForeignKeySymbol, IndexSymbol, MethodSymbol, ParameterSymbol,
    ProjectionSymbol, PropertySymbol, QuerySymbol, RelationshipSymbol, ScopeId, StorageSymbol,
    TriggerSymbol, VariableGlobalSymbol, VariableSymbol, XdataSymbol,
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
    /// Stores methods per class.
    pub methods: HashMap<MethodRef, Method>,
    /// Stores properties in a workspace for public properties.
    pub properties: HashMap<PropertyRef, Property>,
    /// Stores Parameters in a workspace for public parameters.
    pub parameters: HashMap<ParameterRef, Parameter>,
    pub relationships: HashMap<RelationshipRef, Relationship>,
    pub foreignkeys: HashMap<ForeignKeyRef, ForeignKey>,
    pub queries: HashMap<QueryRef, Query>,
    pub indices: HashMap<IndexRef, Index>,
    pub triggers: HashMap<TriggerRef, Trigger>,
    pub xdata: HashMap<XdataRef, XData>,
    pub projections: HashMap<ProjectionRef, Projection>,
    pub storage: HashMap<StorageRef, Storage>,
    /// Stores all local semantic models in a workspace.
    pub lsms: HashMap<ClassId, LocalSemanticModel>,
    /// Stores all class symbols in a workspace.
    pub class_defs: HashMap<ClassId, ClassGlobalSymbol>,
    /// Stores Method Symbols in a workspace for public methods.
    pub method_defs: HashMap<MethodRef, MethodSymbol>,
    /// Stores Property Symbols in a workspace for public properties.
    pub property_defs: HashMap<PropertyRef, PropertySymbol>,
    /// Stores Parameter Symbols in a workspace for public properties.
    pub parameter_defs: HashMap<ParameterRef, ParameterSymbol>,
    pub relationship_defs: HashMap<RelationshipRef, RelationshipSymbol>,
    pub foreignkey_defs: HashMap<ForeignKeyRef, ForeignKeySymbol>,
    pub query_defs: HashMap<QueryRef, QuerySymbol>,
    pub index_defs: HashMap<IndexRef, IndexSymbol>,
    pub trigger_defs: HashMap<TriggerRef, TriggerSymbol>,
    pub xdata_defs: HashMap<XdataRef, XdataSymbol>,
    pub projection_defs: HashMap<ProjectionRef, ProjectionSymbol>,
    pub storage_defs: HashMap<StorageRef, StorageSymbol>,
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
            properties: HashMap::new(),
            property_defs: HashMap::new(),
            parameter_defs: HashMap::new(),
            parameters: HashMap::new(),
            relationships: HashMap::new(),
            foreignkeys: HashMap::new(),
            queries: HashMap::new(),
            indices: HashMap::new(),
            triggers: HashMap::new(),
            xdata: HashMap::new(),
            projections: HashMap::new(),
            storage: HashMap::new(),
            relationship_defs: HashMap::new(),
            foreignkey_defs: HashMap::new(),
            query_defs: HashMap::new(),
            index_defs: HashMap::new(),
            trigger_defs: HashMap::new(),
            xdata_defs: HashMap::new(),
            projection_defs: HashMap::new(),
            storage_defs: HashMap::new(),
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
        var_dependencies: Vec<String>,
        variable_range: Range,
        url: Url,
    ) -> VariableRef {
        if variable.is_public {
            let scopes_to_vars = self.variables.entry(method_ref).or_insert(HashMap::new());
            let vars = scopes_to_vars.entry(scope_id).or_insert(Vec::new());
            let var_ref = VariableRef {
                pub_id: Some(PublicVarId(vars.len())),
                priv_id: None,
            };
            vars.push(variable);
            self.new_variable_symbol(variable_range, url, var_dependencies, method_ref, scope_id);
            return var_ref;
        } else {
            if let Some(lsm) = self.get_local_semantic_mut(&method_ref.class) {
                return lsm.new_variable(method_ref, variable, scope_id);
            }
        }
        eprintln!("Error: failed to add variable");
        return VariableRef {
            pub_id: None,
            priv_id: None,
        };
    }

    /// Given a Property, adds the Property as the value of the `PropertyRef` key
    pub fn new_property(
        &mut self,
        property: Property,
        property_ref: PropertyRef,
        property_range: Range,
        url: Url,
    ) {
        if property.is_public {
            self.new_property_symbol(property.name.clone(), property_range, url, property_ref);
            self.properties.insert(property_ref, property);
        } else {
            if let Some(lsm) = self.get_local_semantic_mut(&property_ref.class) {
                lsm.new_property(property, property_ref);
            }
        }
    }

    // Finds the latest oref definition in a given scope
    pub fn get_oref_in_scope_before_range(
        &self,
        method_ref: MethodRef,
        scope_id: ScopeId,
        variable_name: &str,
        method_call_range: Range,
        private_variable_symbols: &Vec<VariableSymbol>, // the private variable symbols in a scope
    ) -> Option<(Range, String)> {
        let mut variable_definition: Option<Range> = None;
        let mut oref_class: Option<String> = None;
        if let Some(variables) = self
            .variables
            .get(&method_ref)
            .and_then(|scopes| scopes.get(&scope_id))
        {
            let mut potential_variable_indices = HashSet::new();
            for (i, variable) in variables.iter().enumerate() {
                if &variable.name == variable_name
                    && let VariableDefType::OrefDef(oref_cls) = &variable.variable_type
                {
                    potential_variable_indices.insert((i, oref_cls.clone()));
                }
            }
            for (i, oref_cls) in &potential_variable_indices {
                if let Some(variable_def) = self
                    .variable_defs
                    .get(&method_ref)
                    .and_then(|scopes| scopes.get(&scope_id))
                    .and_then(|variables| variables.get(*i))
                {
                    if variable_def.location.end_byte < method_call_range.start_byte {
                        if let Some(curr_var_def) = variable_definition {
                            if variable_def.location.start_byte > curr_var_def.start_byte {
                                variable_definition = Some(variable_def.location);
                                oref_class = Some(oref_cls.clone());
                            }
                        } else {
                            variable_definition = Some(variable_def.location);
                            oref_class = Some(oref_cls.clone());
                        }
                    }
                }
            }
        }
        if variable_definition.is_none() {
            if let Some(lsm) = self.get_local_semantic(&method_ref.class)
                && let Some(variables) = lsm
                    .variables
                    .get(&method_ref)
                    .and_then(|scopes| scopes.get(&scope_id))
            {
                let mut potential_variable_indices = HashSet::new();
                for (i, variable) in variables.iter().enumerate() {
                    if &variable.name == variable_name
                        && let VariableDefType::OrefDef(oref_cls) = &variable.variable_type
                    {
                        potential_variable_indices.insert((i, oref_cls.clone()));
                    }
                }
                for (i, oref_cls) in potential_variable_indices {
                    if let Some(variable_def) = private_variable_symbols.get(i) {
                        if variable_def.location.end_byte < method_call_range.start_byte {
                            if let Some(curr_var_def) = variable_definition {
                                if variable_def.location.start_byte > curr_var_def.start_byte {
                                    variable_definition = Some(variable_def.location);
                                    oref_class = Some(oref_cls);
                                }
                            } else {
                                variable_definition = Some(variable_def.location);
                                oref_class = Some(oref_cls);
                            }
                        }
                    }
                }
            }
        }
        if let Some(var_range) = variable_definition
            && let Some(oref_cls_name) = oref_class
        {
            return Some((var_range, oref_cls_name.clone()));
        }
        None
    }

    /// Given a parameter, adds the parameter as the value of the `ParameterRef` key
    pub fn new_parameter(
        &mut self,
        parameter: Parameter,
        parameter_ref: ParameterRef,
        parameter_range: Range,
        url: Url,
    ) {
        self.new_parameter_symbol(parameter.name.clone(), parameter_range, url, parameter_ref);
        self.parameters.insert(parameter_ref, parameter);
    }

    fn member_symbol(
        name: String,
        location: Range,
        url: Url,
    ) -> crate::scope_structures::MemberSymbol {
        crate::scope_structures::MemberSymbol {
            name,
            url,
            location,
            references: Vec::new(),
        }
    }

    pub fn new_relationship(
        &mut self,
        value: Relationship,
        member_ref: RelationshipRef,
        range: Range,
        url: Url,
    ) {
        let symbol = Self::member_symbol(value.name.clone(), range, url);
        if value.is_public {
            self.relationship_defs.insert(member_ref, symbol);
            self.relationships.insert(member_ref, value);
        } else if let Some(lsm) = self.get_local_semantic_mut(&member_ref.class) {
            lsm.new_relationship(value, member_ref);
        }
    }
    pub fn get_relationship(&self, member_ref: &RelationshipRef) -> Option<&Relationship> {
        self.relationships.get(member_ref).or_else(|| {
            self.get_local_semantic(&member_ref.class)?
                .get_relationship(member_ref)
        })
    }
    pub fn remove_relationship(&mut self, member_ref: &RelationshipRef) -> Option<Relationship> {
        self.relationship_defs.remove(member_ref);
        self.relationships.remove(member_ref).or_else(|| {
            self.get_local_semantic_mut(&member_ref.class)?
                .remove_relationship(member_ref)
        })
    }

    pub fn new_query(&mut self, value: Query, member_ref: QueryRef, range: Range, url: Url) {
        let symbol = Self::member_symbol(value.name.clone(), range, url);
        if value.is_public {
            self.query_defs.insert(member_ref, symbol);
            self.queries.insert(member_ref, value);
        } else if let Some(lsm) = self.get_local_semantic_mut(&member_ref.class) {
            lsm.new_query(value, member_ref);
        }
    }
    pub fn get_query(&self, member_ref: &QueryRef) -> Option<&Query> {
        self.queries.get(member_ref).or_else(|| {
            self.get_local_semantic(&member_ref.class)?
                .get_query(member_ref)
        })
    }
    pub fn remove_query(&mut self, member_ref: &QueryRef) -> Option<Query> {
        self.query_defs.remove(member_ref);
        self.queries.remove(member_ref).or_else(|| {
            self.get_local_semantic_mut(&member_ref.class)?
                .remove_query(member_ref)
        })
    }

    pub fn new_foreignkey(
        &mut self,
        value: ForeignKey,
        member_ref: ForeignKeyRef,
        range: Range,
        url: Url,
    ) {
        self.foreignkey_defs.insert(
            member_ref,
            Self::member_symbol(value.name.clone(), range, url),
        );
        self.foreignkeys.insert(member_ref, value);
    }
    pub fn get_foreignkey(&self, member_ref: &ForeignKeyRef) -> Option<&ForeignKey> {
        self.foreignkeys.get(member_ref)
    }
    pub fn remove_foreignkey(&mut self, member_ref: &ForeignKeyRef) -> Option<ForeignKey> {
        self.foreignkey_defs.remove(member_ref);
        self.foreignkeys.remove(member_ref)
    }
    pub fn new_index(&mut self, value: Index, member_ref: IndexRef, range: Range, url: Url) {
        self.index_defs.insert(
            member_ref,
            Self::member_symbol(value.name.clone(), range, url),
        );
        self.indices.insert(member_ref, value);
    }
    pub fn get_index(&self, member_ref: &IndexRef) -> Option<&Index> {
        self.indices.get(member_ref)
    }
    pub fn remove_index(&mut self, member_ref: &IndexRef) -> Option<Index> {
        self.index_defs.remove(member_ref);
        self.indices.remove(member_ref)
    }
    pub fn new_trigger(&mut self, value: Trigger, member_ref: TriggerRef, range: Range, url: Url) {
        self.trigger_defs.insert(
            member_ref,
            Self::member_symbol(value.name.clone(), range, url),
        );
        self.triggers.insert(member_ref, value);
    }
    pub fn get_trigger(&self, member_ref: &TriggerRef) -> Option<&Trigger> {
        self.triggers.get(member_ref)
    }
    pub fn remove_trigger(&mut self, member_ref: &TriggerRef) -> Option<Trigger> {
        self.trigger_defs.remove(member_ref);
        self.triggers.remove(member_ref)
    }
    pub fn new_xdata(&mut self, value: XData, member_ref: XdataRef, range: Range, url: Url) {
        self.xdata_defs.insert(
            member_ref,
            Self::member_symbol(value.name.clone(), range, url),
        );
        self.xdata.insert(member_ref, value);
    }
    pub fn get_xdata(&self, member_ref: &XdataRef) -> Option<&XData> {
        self.xdata.get(member_ref)
    }
    pub fn remove_xdata(&mut self, member_ref: &XdataRef) -> Option<XData> {
        self.xdata_defs.remove(member_ref);
        self.xdata.remove(member_ref)
    }
    pub fn new_projection(
        &mut self,
        value: Projection,
        member_ref: ProjectionRef,
        range: Range,
        url: Url,
    ) {
        self.projection_defs.insert(
            member_ref,
            Self::member_symbol(value.name.clone(), range, url),
        );
        self.projections.insert(member_ref, value);
    }
    pub fn get_projection(&self, member_ref: &ProjectionRef) -> Option<&Projection> {
        self.projections.get(member_ref)
    }
    pub fn remove_projection(&mut self, member_ref: &ProjectionRef) -> Option<Projection> {
        self.projection_defs.remove(member_ref);
        self.projections.remove(member_ref)
    }
    pub fn new_storage(&mut self, value: Storage, member_ref: StorageRef, range: Range, url: Url) {
        self.storage_defs.insert(
            member_ref,
            Self::member_symbol(value.name.clone(), range, url),
        );
        self.storage.insert(member_ref, value);
    }
    pub fn get_storage(&self, member_ref: &StorageRef) -> Option<&Storage> {
        self.storage.get(member_ref)
    }
    pub fn remove_storage(&mut self, member_ref: &StorageRef) -> Option<Storage> {
        self.storage_defs.remove(member_ref);
        self.storage.remove(member_ref)
    }

    /// Given a Class, adds the class to the `self.classes` vec, returning ClassId, which
    /// corresponds to the index that the Class is stored.
    pub fn new_class(&mut self, class: Class, class_id: ClassId, range: Range, url: Url) {
        self.new_class_symbol(class.name.clone(), range, url, class_id);
        self.classes.insert(class_id, class);
    }
    // TODO ADD METHOD SYMBOL IN NEW_METHOD
    /// Given a Method, adds the method to the vec corresponding to the class the method is defined in.
    pub fn new_method(
        &mut self,
        method: Method,
        method_ref: MethodRef,
        method_range: Range,
        url: Url,
    ) {
        if method.is_public {
            let method_name = method.name.clone();
            self.methods.insert(method_ref, method);
            self.new_method_symbol(method_name, method_range, url, method_ref);
        } else {
            if let Some(lsm) = self.get_local_semantic_mut(&method_ref.class) {
                lsm.new_method(method, method_ref);
            }
        }
    }

    /// Inserts a new `LocalSemanticModel`, hashed by class name, to the global store `self.lsms`
    pub fn new_local_semantic(&mut self, class_id: ClassId, local_semantic: LocalSemanticModel) {
        self.lsms.insert(class_id, local_semantic);
    }

    /// Returns a mutable reference to the local semantic model with the given id.
    ///
    ///  and returns `None` if `lsm_id` is out of bounds.
    pub fn get_local_semantic_mut(
        &mut self,
        class_id: &ClassId,
    ) -> Option<&mut LocalSemanticModel> {
        self.lsms.get_mut(class_id)
    }

    /// Returns an immutable reference to the local semantic model with the given id.
    ///
    ///  and returns `None` if `lsm_id` is out of bounds.
    pub fn get_local_semantic(&self, class_id: &ClassId) -> Option<&LocalSemanticModel> {
        self.lsms.get(class_id)
    }

    /// Returns an immutable reference to the class at `index` in the classes table.
    ///
    /// returns `None` if `index` is out of bounds.
    pub fn get_class(&self, index: &ClassId) -> Option<&Class> {
        self.classes.get(index)
    }

    /// Returns a mutable reference to the class at `index` in the classes table.
    ///
    /// returns `None` if `index` is out of bounds.
    pub fn get_mut_class(&mut self, index: &ClassId) -> Option<&mut Class> {
        self.classes.get_mut(index)
    }

    /// Returns the `ClassGlobalSymbol` at `index` in the class symbol table.
    ///
    /// and returns `None` if `index` is out of bounds.
    pub fn get_class_symbol(&self, index: &ClassId) -> Option<&ClassGlobalSymbol> {
        self.class_defs.get(index)
    }

    /// Returns a mutable ref to `ClassGlobalSymbol` at `index` in the class symbol table.
    ///
    /// and returns `None` if `index` is out of bounds.
    pub fn get_class_symbol_mut(&mut self, index: &ClassId) -> Option<&mut ClassGlobalSymbol> {
        self.class_defs.get_mut(index)
    }

    /// Fetches a mutable reference to a method by `MethodRef`.
    ///
    /// Returns a mutable reference to the `method` corresponding to `MethodRef` if it exists, None otherwise.
    pub fn get_mut_method(&mut self, method_ref: &MethodRef) -> Option<&mut Method> {
        if self.methods.contains_key(method_ref) {
            return self.methods.get_mut(method_ref);
        }
        if let Some(lsm) = self.lsms.get_mut(&method_ref.class) {
            return lsm.get_method_mut(method_ref);
        }
        None
    }

    /// Fetches an immutable reference to a method by `MethodRef`.
    ///
    /// Returns an immutable referencce to the `method` corresponding to `MethodRef` if it exists, None otherwise.
    pub fn get_method(&self, method_ref: &MethodRef) -> Option<&Method> {
        if let Some(method) = self.methods.get(method_ref) {
            return Some(method);
        } else if let Some(lsm) = self.get_local_semantic(&method_ref.class) {
            return lsm.get_method(method_ref);
        }
        return None;
    }

    /// Removes  `method` corresponding to `MethodRef` and returns it if it exists, None otherwise.
    pub fn remove_method(&mut self, method_ref: &MethodRef) -> Option<Method> {
        self.method_defs.remove(method_ref);
        self.variable_defs.remove(method_ref);
        self.variables.remove(method_ref);
        if let Some(lsm) = self.get_local_semantic_mut(&method_ref.class) {
            if let Some(method) = lsm.remove_method(method_ref) {
                return Some(method);
            }
        }
        self.methods.remove(method_ref)
    }

    /// Removes `class` corresponding to `class_id`.
    pub fn remove_class(&mut self, class_id: &ClassId) {
        self.classes.remove(class_id);
    }

    /// Removes  `property` corresponding to `PropertyRef` and returns it if it exists, None otherwise.
    pub fn remove_property(&mut self, property_ref: &PropertyRef) -> Option<Property> {
        self.property_defs.remove(property_ref);
        if let Some(property) = self.properties.remove(property_ref) {
            return Some(property);
        } else {
            if let Some(lsm) = self.get_local_semantic_mut(&property_ref.class) {
                return lsm.remove_property(property_ref);
            }
        }
        return None;
    }

    /// Returns the method symbol if it is now private. This will be added then to the scope tree.
    pub fn change_method_publicity(
        &mut self,
        method_ref: &MethodRef,
        method_range: Range,
        url: Url,
    ) -> Option<MethodSymbol> {
        if let Some(method) = self.remove_method(method_ref) {
            if let Some(lsm) = self.get_local_semantic_mut(&method_ref.class) {
                lsm.new_method(method, *method_ref);
            }
            return self.method_defs.remove(method_ref);
        } else if let Some(lsm) = self.get_local_semantic_mut(&method_ref.class) {
            if let Some(method) = lsm.remove_method(&method_ref) {
                self.new_method(method, *method_ref, method_range, url);
            }
        }
        return None;
    }

    /// Removes  `parameter` corresponding to `ParameterRef` and returns it if it exists, None otherwise.
    pub fn remove_parameter(&mut self, parameter_ref: &ParameterRef) -> Option<Parameter> {
        self.parameter_defs.remove(parameter_ref);
        self.parameters.remove(parameter_ref)
    }

    /// Fetches a mutable reference to a parameter by `ParameterRef`.
    ///
    /// Returns a mutable reference to the `parameter` corresponding to `ParameterRef` if it exists, None otherwise.
    pub fn get_mut_parameter(&mut self, parameter_ref: &ParameterRef) -> Option<&mut Parameter> {
        self.parameters.get_mut(parameter_ref)
    }

    /// Fetches an immutable reference to a parameter by `ParameterRef`.
    ///
    /// Returns an immutable referencce to the `parameter` corresponding to `ParameterRef` if it exists, None otherwise.
    pub fn get_parameter(&self, parameter_ref: &ParameterRef) -> Option<&Parameter> {
        self.parameters.get(parameter_ref)
    }

    /// Fetches a mutable reference to a ParameterSymbol by `ParameterRef`.
    ///
    /// Returns a mutable reference to the `ParameterSymbol` corresponding to `ParameterRef` if it exists, None otherwise.
    pub fn get_mut_parameter_symbol(
        &mut self,
        parameter_ref: &ParameterRef,
    ) -> Option<&mut ParameterSymbol> {
        self.parameter_defs.get_mut(parameter_ref)
    }

    /// Fetches an immutable reference to a parameter symbol by `ParameterRef`.
    ///
    /// Returns an immutable referencce to the `ParameterSymbol` corresponding to `ParameterRef` if it exists, None otherwise.
    pub fn get_parameter_symbol(&self, parameter_ref: &ParameterRef) -> Option<&ParameterSymbol> {
        self.parameter_defs.get(parameter_ref)
    }

    /// Fetches a mutable reference to a property by `PropertyRef`.
    ///
    /// Looks up the corresponding property for `property_ref` and then indexes into it. Logs and returns `None`
    /// if the class has no recorded property for `PropertyRef`.
    pub fn get_mut_property(&mut self, property_ref: &PropertyRef) -> Option<&mut Property> {
        self.properties.get_mut(property_ref)
    }

    /// Fetches an immutable reference to a property by `PropertyRef`.
    ///
    /// Looks up the corresponding property for `property_ref` and then indexes into it. Logs and returns `None`
    /// if the class has no recorded property for `PropertyRef`.
    pub fn get_property(&self, property_ref: &PropertyRef) -> Option<&Property> {
        self.properties.get(property_ref)
    }

    /// Returns a mutable ref to the `PropertySymbol` for a class symbol by symbol index.
    ///
    /// Logs and returns `None` if the class has no property symbols recorded or `property_symbol_id` is
    /// out of bounds.
    pub fn get_property_symbol_mut(
        &mut self,
        property_symbol_ref: &PropertyRef,
    ) -> Option<&mut PropertySymbol> {
        self.property_defs.get_mut(property_symbol_ref)
    }

    /// Returns an immutable ref to the `PropertySymbol` for a class symbol by symbol index.
    ///
    /// Logs and returns `None` if the class has no property symbols recorded or `property_symbol_id` is
    /// out of bounds.
    pub fn get_property_symbol(
        &self,
        property_symbol_ref: &PropertyRef,
    ) -> Option<&PropertySymbol> {
        self.property_defs.get(property_symbol_ref)
    }

    /// Returns a mutable ref to the `MethodSymbol` for a class symbol by symbol index.
    ///
    /// Logs and returns `None` if the class has no method symbols recorded or `method_symbol_id` is
    /// out of bounds.
    pub fn get_method_symbol_mut(
        &mut self,
        method_symbol_ref: &MethodRef,
    ) -> Option<&mut MethodSymbol> {
        self.method_defs.get_mut(method_symbol_ref)
    }

    /// Returns an immutable ref to the `MethodSymbol` for a class symbol by symbol index.
    ///
    /// Logs and returns `None` if the class has no method symbols recorded or `method_symbol_id` is
    /// out of bounds.
    pub fn get_method_symbol(&self, method_symbol_ref: &MethodRef) -> Option<&MethodSymbol> {
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

    pub fn reset_method_semantics(&mut self, method_ref: &MethodRef) {
        self.variables.remove(method_ref);
        self.variable_defs.remove(method_ref);
        if let Some(lsm) = self.get_local_semantic_mut(&method_ref.class) {
            lsm.variables.remove(method_ref);
        }
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
        methods_to_remove: HashSet<MethodRef>,
        properties_to_remove: HashSet<PropertyRef>,
        parameters_to_remove: HashSet<ParameterRef>,
    ) {
        for method_ref in &methods_to_remove {
            self.remove_method(method_ref);
        }
        for property_ref in &properties_to_remove {
            self.remove_property(property_ref);
        }
        for parameter_ref in &parameters_to_remove {
            self.remove_parameter(parameter_ref);
        }

        // reset everything in the local semantic model
        if let Some(local_semantic_model) = self.lsms.get_mut(class_id) {
            local_semantic_model.partial_clear(methods_to_remove, properties_to_remove);
        };
    }

    /// Clears all semantic state associated with a re-parsed document.
    /// Marks the class symbol as inactive and removes all method/variable symbols for the document.
    ///
    /// Resets the class entry, removes method/variable tables for `class_id`, and clears the
    /// associated local semantic model. Use this when a document is being reparsed, not deleted.
    pub fn reset_doc(
        &mut self,
        class_id: &ClassId,
        class_name: String,
    ) -> (
        HashMap<String, MethodRef>,
        HashMap<String, PropertyRef>,
        HashMap<String, ParameterRef>,
    ) {
        let Some(class) = self.classes.get_mut(class_id) else {
            eprintln!("Error: class named {:?} not found", class_name);
            return (HashMap::new(), HashMap::new(), HashMap::new());
        };

        let old_methods = class.methods.clone();
        let old_properties = class.properties.clone();
        let old_parameters = class.parameters.clone();
        for method_ref in class.methods.values() {
            self.methods.remove(method_ref);
            self.method_defs.remove(method_ref);
            self.variables.remove(method_ref);
            self.variable_defs.remove(method_ref);
        }
        for property_ref in class.properties.values() {
            self.properties.remove(property_ref);
            self.property_defs.remove(property_ref);
        }
        for parameter_ref in class.parameters.values() {
            self.parameter_defs.remove(parameter_ref);
            self.parameters.remove(parameter_ref);
        }
        class.clear(class_name.clone(), true);

        // reset everything in the local semantic model
        if let Some(local_semantic_model) = self.lsms.get_mut(&class_id) {
            local_semantic_model.clear();
        };
        let Some(class_symbol) = self.class_defs.get_mut(class_id) else {
            eprintln!("Error: in reset_doc, class symbol not found");
            return (old_methods, old_properties, old_parameters);
        };
        class_symbol.alive = false;
        (old_methods, old_properties, old_parameters)
    }

    pub fn next_id(&mut self) -> usize {
        let id = self.next_class_id;
        self.next_class_id += 1;
        id
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

    /// Creates a new class symbol entry.
    fn new_class_symbol(&mut self, name: String, range: Range, url: Url, symbol_id: ClassId) {
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

    /// Adds a new property symbol for PropertyRef.
    fn new_property_symbol(
        &mut self,
        name: String,
        range: Range,
        url: Url,
        property_symbol_ref: PropertyRef,
    ) {
        let property_symbol = PropertySymbol {
            name,
            url,
            location: range,
            references: Vec::new(),
        };
        self.property_defs
            .insert(property_symbol_ref, property_symbol);
    }

    /// Adds a new Parameter symbol for ParameterRef.
    fn new_parameter_symbol(
        &mut self,
        name: String,
        range: Range,
        url: Url,
        parameter_symbol_ref: ParameterRef,
    ) {
        let parameter_symbol = ParameterSymbol {
            name,
            url,
            location: range,
            references: Vec::new(),
        };
        self.parameter_defs
            .insert(parameter_symbol_ref, parameter_symbol);
    }

    /// Adds a new method symbol for `MethodRef`.
    fn new_method_symbol(
        &mut self,
        name: String,
        range: Range,
        url: Url,
        method_symbol_ref: MethodRef,
    ) {
        let method_symbol = MethodSymbol {
            name,
            url,
            location: range,
            references: Vec::new(),
            method_dependencies: Vec::new(),
        };
        self.method_defs.insert(method_symbol_ref, method_symbol);
    }

    /// Adds a new variable symbol to the vec under `MethodRef`.
    fn new_variable_symbol(
        &mut self,
        range: Range,
        url: Url,
        var_dependencies: Vec<String>,
        method_symbol_ref: MethodRef,
        scope_id: ScopeId,
    ) {
        let scopes_to_vars = self
            .variable_defs
            .entry(method_symbol_ref)
            .or_insert(HashMap::new());
        scopes_to_vars
            .entry(scope_id)
            .or_insert(Vec::new())
            .push(VariableGlobalSymbol {
                url,
                location: range,
                var_dependencies,
            });
    }

    /// Build a reverse inheritance index for each class.
    ///
    /// For every class, this returns the transitive set of subclasses that depend on it via
    /// `Extends`. Inactive classes are skipped.
    pub fn build_dependents(&self, name_to_id: &HashMap<String, ClassId>) -> Dependents {
        let mut entries: Vec<(ClassId, &Class)> =
            self.classes.iter().map(|(&id, c)| (id, c)).collect();
        entries.sort_by_key(|(id, _)| id.0);

        let id_to_idx: HashMap<ClassId, usize> = entries
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();

        let n = entries.len();
        let mut children = vec![Vec::new(); n];
        let mut index = Dependents::new();

        for (child_id, cls) in entries.iter() {
            if !cls.active {
                continue;
            }
            for (parent_name, parent_range) in &cls.inherited_classes {
                if let Some(&parent_id) = name_to_id.get(parent_name) {
                    if let Some(&parent_idx) = id_to_idx.get(&parent_id) {
                        if entries[parent_idx].1.active && entries[parent_idx].1.is_final != true {
                            children[parent_idx].push((*child_id, parent_range.clone()));
                        }
                    }
                }
            }
        }

        for (i, (class_id, cls)) in entries.iter().enumerate() {
            if !cls.active {
                continue;
            }
            let direct = children[i].iter().cloned().collect();
            index.direct_subclasses.insert(*class_id, direct);
        }

        let mut memo: Vec<Option<HashSet<ClassId>>> = vec![None; n];
        let mut state: Vec<DfsState> = vec![DfsState::Unvisited; n];

        fn dfs(
            idx: usize,
            children: &Vec<Vec<(ClassId, tower_lsp::lsp_types::Range)>>,
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
            for (child, _) in &children[idx] {
                table.insert(*child);
                if let Some(&child_idx) = id_to_idx.get(child) {
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
            let dependents: HashSet<ClassId> = dfs(i, &children, &id_to_idx, &mut memo, &mut state)
                .into_iter()
                .collect();
            index.dependent_classes.insert(*class_id, dependents);
        }

        index
    }
}
