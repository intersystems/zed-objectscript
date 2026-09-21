use crate::document::Document;
use crate::parse_structures::{
    CodeMode, Language, Method, MethodType, Parameter, Property, TypeName, VariableDefType,
};
use crate::workspace::ProjectData;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueChange<T> {
    pub before: T,
    pub after: T,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemberChanges<T> {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: BTreeMap<String, T>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectionChanges<T> {
    pub added: Vec<T>,
    pub removed: Vec<T>,
}

impl<T> Default for CollectionChanges<T> {
    fn default() -> Self {
        Self {
            added: Vec::new(),
            removed: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariableSnapshot {
    pub name: String,
    pub argument_type: Option<TypeName>,
    pub is_public: bool,
    pub variable_type: VariableDefType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MethodSnapshot {
    pub method_type: MethodType,
    pub return_type: Option<TypeName>,
    pub is_public: bool,
    pub procedure_block: Option<bool>,
    pub language: Option<Language>,
    pub code_mode: CodeMode,
    pub public_variables_declared: Vec<String>,
    pub final_keyword: Option<bool>,
    pub variables: Vec<VariableSnapshot>,
    /// Exact declaration/body source for implementation-level comparison.
    pub implementation: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertySnapshot {
    pub required: bool,
    pub is_public: bool,
    pub final_keyword: Option<bool>,
    pub multidimensional: bool,
    pub return_type: Option<TypeName>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterSnapshot {
    pub final_keyword: Option<bool>,
    pub return_type: Option<TypeName>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassSnapshot {
    pub name: String,
    pub imports: Vec<String>,
    /// Parent order is significant for ObjectScript multiple inheritance.
    pub inherited_classes: Vec<String>,
    pub unresolved_inherited_classes: BTreeSet<String>,
    pub inheritance_direction: Option<String>,
    pub procedure_block: Option<bool>,
    pub language: Option<Language>,
    pub final_keyword: Option<bool>,
    pub methods: BTreeMap<String, MethodSnapshot>,
    pub properties: BTreeMap<String, PropertySnapshot>,
    pub parameters: BTreeMap<String, ParameterSnapshot>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MethodDiff {
    pub method_type: Option<ValueChange<MethodType>>,
    pub return_type: Option<ValueChange<Option<TypeName>>>,
    pub is_public: Option<ValueChange<bool>>,
    pub procedure_block: Option<ValueChange<Option<bool>>>,
    pub language: Option<ValueChange<Option<Language>>>,
    pub code_mode: Option<ValueChange<CodeMode>>,
    pub public_variables_declared: Option<ValueChange<Vec<String>>>,
    pub final_keyword: Option<ValueChange<Option<bool>>>,
    pub variables: CollectionChanges<VariableSnapshot>,
    /// Exact method declaration/body source changed.
    pub source_changed: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PropertyDiff {
    pub required: Option<ValueChange<bool>>,
    pub is_public: Option<ValueChange<bool>>,
    pub final_keyword: Option<ValueChange<Option<bool>>>,
    pub multidimensional: Option<ValueChange<bool>>,
    pub return_type: Option<ValueChange<Option<TypeName>>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParameterDiff {
    pub final_keyword: Option<ValueChange<Option<bool>>>,
    pub return_type: Option<ValueChange<Option<TypeName>>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClassDiff {
    pub class_name: String,
    pub imports: Option<ValueChange<Vec<String>>>,
    pub inherited_classes: Option<ValueChange<Vec<String>>>,
    pub unresolved_inherited_classes: Option<ValueChange<BTreeSet<String>>>,
    pub inheritance_direction: Option<ValueChange<Option<String>>>,
    pub procedure_block: Option<ValueChange<Option<bool>>>,
    pub language: Option<ValueChange<Option<Language>>>,
    pub final_keyword: Option<ValueChange<Option<bool>>>,
    pub methods: MemberChanges<MethodDiff>,
    pub properties: MemberChanges<PropertyDiff>,
    pub parameters: MemberChanges<ParameterDiff>,
    /// Source changed, but no difference is represented by the current model.
    pub unmodeled_source_change: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClassComparison {
    Unchanged {
        class_name: String,
    },
    Added {
        class_name: String,
    },
    Removed {
        class_name: String,
    },
    SnapshotUnavailable {
        class_name: String,
        baseline_available: bool,
        target_available: bool,
    },
    Changed(ClassDiff),
}

impl ClassComparison {
    pub fn class_name(&self) -> &str {
        match self {
            Self::Unchanged { class_name }
            | Self::Added { class_name }
            | Self::Removed { class_name }
            | Self::SnapshotUnavailable { class_name, .. } => class_name,
            Self::Changed(diff) => &diff.class_name,
        }
    }
}

pub fn compare_classes_parallel(
    class_names: &[String],
    baseline: &ProjectData,
    target: &ProjectData,
) -> Vec<ClassComparison> {
    class_names
        .par_iter()
        .map(|class_name| compare_class(baseline, target, class_name))
        .collect()
}

/// Compare every class name present in either workspace.
///
/// The union ensures classes that exist in only one workspace are reported as
/// added or removed. Result order is unspecified; callers that need a stable
/// report should sort by class name at the presentation boundary.
pub fn compare_workspaces_parallel(
    baseline: &ProjectData,
    target: &ProjectData,
) -> Vec<ClassComparison> {
    let class_names: Vec<String> = baseline
        .classes
        .keys()
        .chain(target.classes.keys())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    compare_classes_parallel(&class_names, baseline, target)
}

pub fn compare_class(
    baseline: &ProjectData,
    target: &ProjectData,
    class_name: &str,
) -> ClassComparison {
    let baseline_document = class_document(baseline, class_name);
    let target_document = class_document(target, class_name);
    match (baseline_document, target_document) {
        (None, None) => ClassComparison::Unchanged {
            class_name: class_name.to_string(),
        },
        (None, Some(_)) => ClassComparison::Added {
            class_name: class_name.to_string(),
        },
        (Some(_), None) => ClassComparison::Removed {
            class_name: class_name.to_string(),
        },
        (Some(before), Some(after)) if before.content == after.content => {
            let unresolved_before = unresolved_inherited_classes(baseline, class_name);
            let unresolved_after = unresolved_inherited_classes(target, class_name);
            if matches!(
                (&unresolved_before, &unresolved_after),
                (Some(before), Some(after)) if before == after
            ) {
                ClassComparison::Unchanged {
                    class_name: class_name.to_string(),
                }
            } else {
                compare_snapshots(baseline, target, class_name)
            }
        }
        (Some(_), Some(_)) => compare_snapshots(baseline, target, class_name),
    }
}

fn compare_snapshots(
    baseline: &ProjectData,
    target: &ProjectData,
    class_name: &str,
) -> ClassComparison {
    let before = snapshot_class(baseline, class_name);
    let after = snapshot_class(target, class_name);
    let baseline_available = before.is_some();
    let target_available = after.is_some();
    let (Some(before), Some(after)) = (before, after) else {
        return ClassComparison::SnapshotUnavailable {
            class_name: class_name.to_string(),
            baseline_available,
            target_available,
        };
    };
    ClassComparison::Changed(diff_class(before, after))
}

fn unresolved_inherited_classes(data: &ProjectData, class_name: &str) -> Option<BTreeSet<String>> {
    let class_id = data.classes.get(class_name)?;
    let class = data.global_semantic_model.get_class(class_id)?;
    Some(
        class
            .inherited_classes
            .iter()
            .filter_map(|(parent_name, _)| {
                (!data.classes.contains_key(parent_name)).then_some(parent_name.clone())
            })
            .collect(),
    )
}

pub fn snapshot_class(data: &ProjectData, class_name: &str) -> Option<ClassSnapshot> {
    let class_id = *data.classes.get(class_name)?;
    let class = data.global_semantic_model.get_class(&class_id)?;
    let document = class_document(data, class_name)?;

    let mut imports = class.imports.clone();
    imports.sort();
    let inherited_classes = class
        .inherited_classes
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let unresolved_inherited_classes = unresolved_inherited_classes(data, class_name)?;

    let methods = class
        .methods
        .iter()
        .filter_map(|(name, method_ref)| {
            let method = data.global_semantic_model.get_method(method_ref)?;
            let implementation = data
                .global_semantic_model
                .get_method_symbol(method_ref)
                .or_else(|| document.scope_tree.private_method_defs.get(method_ref))
                .and_then(|symbol| {
                    document
                        .content
                        .get(symbol.location.start_byte..symbol.location.end_byte)
                })
                .map(str::to_owned);
            Some((
                name.clone(),
                snapshot_method(data, method_ref, method, implementation),
            ))
        })
        .collect();

    let local = data.global_semantic_model.get_local_semantic(&class_id);
    let properties = class
        .properties
        .iter()
        .filter_map(|(name, property_ref)| {
            data.global_semantic_model
                .get_property(property_ref)
                .or_else(|| local.and_then(|model| model.get_property(property_ref)))
                .map(|property| (name.clone(), snapshot_property(property)))
        })
        .collect();
    let parameters = class
        .parameters
        .iter()
        .filter_map(|(name, parameter_ref)| {
            data.global_semantic_model
                .get_parameter(parameter_ref)
                .map(|parameter| (name.clone(), snapshot_parameter(parameter)))
        })
        .collect();

    Some(ClassSnapshot {
        name: class.name.clone(),
        imports,
        inherited_classes,
        unresolved_inherited_classes,
        inheritance_direction: class.inheritance_direction.clone(),
        procedure_block: class.is_procedure_block,
        language: class.default_language.clone(),
        final_keyword: class.is_final,
        methods,
        properties,
        parameters,
    })
}

fn class_document<'a>(data: &'a ProjectData, class_name: &str) -> Option<&'a Document> {
    let class_id = data.classes.get(class_name)?;
    let symbol = data.global_semantic_model.get_class_symbol(class_id)?;
    data.documents.get(&symbol.url)
}

fn snapshot_method(
    data: &ProjectData,
    method_ref: &crate::parse_structures::MethodRef,
    method: &Method,
    implementation: Option<String>,
) -> MethodSnapshot {
    let local = data
        .global_semantic_model
        .get_local_semantic(&method_ref.class);
    let mut variables = Vec::new();
    for refs in method.variables.values() {
        for (variable_ref, scope_id) in refs {
            let variable = if let Some(public_id) = variable_ref.pub_id {
                data.global_semantic_model
                    .get_variable(method_ref, public_id.0, scope_id)
            } else if let Some(private_id) = variable_ref.priv_id {
                local.and_then(|model| model.get_variable(method_ref, private_id.0, scope_id))
            } else {
                None
            };
            if let Some(variable) = variable {
                variables.push(VariableSnapshot {
                    name: variable.name.clone(),
                    argument_type: variable.arg_type.clone(),
                    is_public: variable.is_public,
                    variable_type: variable.variable_type.clone(),
                });
            }
        }
    }
    let mut public_variables_declared: Vec<_> =
        method.public_variables_declared.iter().cloned().collect();
    public_variables_declared.sort();
    MethodSnapshot {
        method_type: method.method_type,
        return_type: method.return_type.clone(),
        is_public: method.is_public,
        procedure_block: method.is_procedure_block,
        language: method.language.clone(),
        code_mode: method.code_mode.clone(),
        public_variables_declared,
        final_keyword: method.is_final,
        variables,
        implementation,
    }
}

fn snapshot_property(property: &Property) -> PropertySnapshot {
    PropertySnapshot {
        required: property.required,
        is_public: property.is_public,
        final_keyword: property.is_final,
        multidimensional: property.multidimensional,
        return_type: property.return_type.clone(),
    }
}

fn snapshot_parameter(parameter: &Parameter) -> ParameterSnapshot {
    ParameterSnapshot {
        final_keyword: parameter.is_final,
        return_type: parameter.return_type.clone(),
    }
}

fn diff_class(before: ClassSnapshot, after: ClassSnapshot) -> ClassDiff {
    let methods = diff_members(&before.methods, &after.methods, diff_method);
    let properties = diff_members(&before.properties, &after.properties, diff_property);
    let parameters = diff_members(&before.parameters, &after.parameters, diff_parameter);
    let mut result = ClassDiff {
        class_name: before.name.clone(),
        imports: value_change(&before.imports, &after.imports),
        inherited_classes: value_change(&before.inherited_classes, &after.inherited_classes),
        unresolved_inherited_classes: value_change(
            &before.unresolved_inherited_classes,
            &after.unresolved_inherited_classes,
        ),
        inheritance_direction: value_change(
            &before.inheritance_direction,
            &after.inheritance_direction,
        ),
        procedure_block: value_change(&before.procedure_block, &after.procedure_block),
        language: value_change(&before.language, &after.language),
        final_keyword: value_change(&before.final_keyword, &after.final_keyword),
        methods,
        properties,
        parameters,
        unmodeled_source_change: false,
    };
    result.unmodeled_source_change = !result.has_semantic_changes();
    result
}

impl ClassDiff {
    pub fn has_semantic_changes(&self) -> bool {
        self.imports.is_some()
            || self.inherited_classes.is_some()
            || self.unresolved_inherited_classes.is_some()
            || self.inheritance_direction.is_some()
            || self.procedure_block.is_some()
            || self.language.is_some()
            || self.final_keyword.is_some()
            || has_member_changes(&self.methods)
            || has_member_changes(&self.properties)
            || has_member_changes(&self.parameters)
    }
}

fn diff_method(before: &MethodSnapshot, after: &MethodSnapshot) -> Option<MethodDiff> {
    let variables = multiset_changes(&before.variables, &after.variables);
    let result = MethodDiff {
        method_type: value_change(&before.method_type, &after.method_type),
        return_type: value_change(&before.return_type, &after.return_type),
        is_public: value_change(&before.is_public, &after.is_public),
        procedure_block: value_change(&before.procedure_block, &after.procedure_block),
        language: value_change(&before.language, &after.language),
        code_mode: value_change(&before.code_mode, &after.code_mode),
        public_variables_declared: value_change(
            &before.public_variables_declared,
            &after.public_variables_declared,
        ),
        final_keyword: value_change(&before.final_keyword, &after.final_keyword),
        variables,
        source_changed: before.implementation != after.implementation,
    };
    (result.method_type.is_some()
        || result.return_type.is_some()
        || result.is_public.is_some()
        || result.procedure_block.is_some()
        || result.language.is_some()
        || result.code_mode.is_some()
        || result.public_variables_declared.is_some()
        || result.final_keyword.is_some()
        || !result.variables.added.is_empty()
        || !result.variables.removed.is_empty()
        || result.source_changed)
        .then_some(result)
}

fn diff_property(before: &PropertySnapshot, after: &PropertySnapshot) -> Option<PropertyDiff> {
    let result = PropertyDiff {
        required: value_change(&before.required, &after.required),
        is_public: value_change(&before.is_public, &after.is_public),
        final_keyword: value_change(&before.final_keyword, &after.final_keyword),
        multidimensional: value_change(&before.multidimensional, &after.multidimensional),
        return_type: value_change(&before.return_type, &after.return_type),
    };
    (result.required.is_some()
        || result.is_public.is_some()
        || result.final_keyword.is_some()
        || result.multidimensional.is_some()
        || result.return_type.is_some())
    .then_some(result)
}

fn diff_parameter(before: &ParameterSnapshot, after: &ParameterSnapshot) -> Option<ParameterDiff> {
    let result = ParameterDiff {
        final_keyword: value_change(&before.final_keyword, &after.final_keyword),
        return_type: value_change(&before.return_type, &after.return_type),
    };
    (result.final_keyword.is_some() || result.return_type.is_some()).then_some(result)
}

fn diff_members<T, D>(
    before: &BTreeMap<String, T>,
    after: &BTreeMap<String, T>,
    compare: impl Fn(&T, &T) -> Option<D>,
) -> MemberChanges<D> {
    let before_names: BTreeSet<_> = before.keys().cloned().collect();
    let after_names: BTreeSet<_> = after.keys().cloned().collect();
    let added = after_names.difference(&before_names).cloned().collect();
    let removed = before_names.difference(&after_names).cloned().collect();
    let changed = before_names
        .intersection(&after_names)
        .filter_map(|name| compare(&before[name], &after[name]).map(|diff| (name.clone(), diff)))
        .collect();
    MemberChanges {
        added,
        removed,
        changed,
    }
}

fn value_change<T: Clone + Eq>(before: &T, after: &T) -> Option<ValueChange<T>> {
    (before != after).then(|| ValueChange {
        before: before.clone(),
        after: after.clone(),
    })
}

fn multiset_changes<T: Clone + Eq>(before: &[T], after: &[T]) -> CollectionChanges<T> {
    let mut matched_after = vec![false; after.len()];
    let mut removed = Vec::new();
    for value in before {
        if let Some((index, _)) = after
            .iter()
            .enumerate()
            .find(|(index, candidate)| !matched_after[*index] && *candidate == value)
        {
            matched_after[index] = true;
        } else {
            removed.push(value.clone());
        }
    }
    let added = after
        .iter()
        .zip(matched_after)
        .filter_map(|(value, matched)| (!matched).then(|| value.clone()))
        .collect();
    CollectionChanges { added, removed }
}

fn has_member_changes<T>(changes: &MemberChanges<T>) -> bool {
    !changes.added.is_empty() || !changes.removed.is_empty() || !changes.changed.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_structures::FileType;
    use crate::workspace::ProjectState;
    use tower_lsp::lsp_types::Url;

    const BEFORE: &str = r#"
Class Demo.Compare [ Not ProcedureBlock ]
{
Property Status As %String;

ClassMethod Calculate() As %Integer
{
    set oldVar = 1
    quit oldVar
}
}
"#;

    const AFTER: &str = r#"
Class Demo.Compare [ Final, Not ProcedureBlock ]
{
Property Status As %Integer [ Required ];
Property Added As %String;

ClassMethod Calculate() As %String
{
    set newVar = 1
    quit newVar
}
}
"#;

    fn project(content: &str) -> ProjectState {
        let project = ProjectState::new();
        project.handle_document_opened(
            Url::parse("file:///workspace/Demo.Compare.cls").unwrap(),
            content.to_string(),
            FileType::Cls,
            1,
        );
        project
    }

    #[test]
    fn identical_source_short_circuits_to_unchanged() {
        let baseline = project(BEFORE);
        let target = project(BEFORE);
        let baseline = baseline.data.read();
        let target = target.data.read();
        assert_eq!(
            compare_class(&baseline, &target, "Demo.Compare"),
            ClassComparison::Unchanged {
                class_name: "Demo.Compare".to_string()
            }
        );
    }

    #[test]
    fn reports_granular_class_member_and_variable_changes() {
        let baseline = project(BEFORE);
        let target = project(AFTER);
        let baseline = baseline.data.read();
        let target = target.data.read();
        let ClassComparison::Changed(diff) = compare_class(&baseline, &target, "Demo.Compare")
        else {
            panic!("expected a changed class");
        };

        assert_eq!(
            diff.final_keyword,
            Some(ValueChange {
                before: None,
                after: Some(true),
            })
        );
        assert_eq!(diff.properties.added, vec!["Added"]);
        let status = diff
            .properties
            .changed
            .get("Status")
            .expect("Status property should change");
        assert!(status.return_type.is_some());

        let method = diff
            .methods
            .changed
            .get("Calculate")
            .expect("Calculate method should change");
        assert!(method.return_type.is_some());
        assert!(method.source_changed);
        assert!(
            method
                .variables
                .removed
                .iter()
                .any(|variable| variable.name == "oldVar")
        );
        assert!(
            method
                .variables
                .added
                .iter()
                .any(|variable| variable.name == "newVar")
        );
        assert!(!diff.unmodeled_source_change);
    }

    #[test]
    fn compares_union_of_both_workspaces() {
        fn add_class(project: &ProjectState, class_name: &str, content: &str) {
            project.handle_document_opened(
                Url::parse(&format!("file:///workspace/{class_name}.cls")).unwrap(),
                content.to_string(),
                FileType::Cls,
                1,
            );
        }

        let baseline = ProjectState::new();
        add_class(&baseline, "Demo.Compare", BEFORE);
        add_class(
            &baseline,
            "Demo.Unchanged",
            "Class Demo.Unchanged { Property Value As %String; }",
        );
        add_class(
            &baseline,
            "Demo.Removed",
            "Class Demo.Removed { Property Value As %String; }",
        );

        let target = ProjectState::new();
        add_class(&target, "Demo.Compare", AFTER);
        add_class(
            &target,
            "Demo.Unchanged",
            "Class Demo.Unchanged { Property Value As %String; }",
        );
        add_class(
            &target,
            "Demo.Added",
            "Class Demo.Added { Property Value As %String; }",
        );

        let baseline = baseline.data.read();
        let target = target.data.read();
        let comparisons = compare_workspaces_parallel(&baseline, &target);

        assert!(comparisons.iter().any(|comparison| matches!(
            comparison,
            ClassComparison::Changed(diff) if diff.class_name == "Demo.Compare"
        )));
        assert!(comparisons.iter().any(|comparison| matches!(
            comparison,
            ClassComparison::Unchanged { class_name }
                if class_name == "Demo.Unchanged"
        )));
        assert!(comparisons.iter().any(|comparison| matches!(
            comparison,
            ClassComparison::Added { class_name } if class_name == "Demo.Added"
        )));
        assert!(comparisons.iter().any(|comparison| matches!(
            comparison,
            ClassComparison::Removed { class_name } if class_name == "Demo.Removed"
        )));
        assert_eq!(comparisons.len(), 4);
    }

    #[test]
    fn reports_inheritance_resolution_change_with_identical_source() {
        const CHILD: &str = "Class Demo.Child Extends Demo.Parent { }";

        let baseline = ProjectState::new();
        baseline.handle_document_opened(
            Url::parse("file:///workspace/Demo.Child.cls").unwrap(),
            CHILD.to_string(),
            FileType::Cls,
            1,
        );

        let target = ProjectState::new();
        target.handle_document_opened(
            Url::parse("file:///workspace/Demo.Child.cls").unwrap(),
            CHILD.to_string(),
            FileType::Cls,
            1,
        );
        target.handle_document_opened(
            Url::parse("file:///workspace/Demo.Parent.cls").unwrap(),
            "Class Demo.Parent { }".to_string(),
            FileType::Cls,
            1,
        );

        let baseline = baseline.data.read();
        let target = target.data.read();
        let ClassComparison::Changed(diff) = compare_class(&baseline, &target, "Demo.Child") else {
            panic!("inheritance resolution change should be reported");
        };

        assert_eq!(
            diff.unresolved_inherited_classes,
            Some(ValueChange {
                before: BTreeSet::from(["Demo.Parent".to_string()]),
                after: BTreeSet::new(),
            })
        );
        assert!(!diff.unmodeled_source_change);
    }
}
