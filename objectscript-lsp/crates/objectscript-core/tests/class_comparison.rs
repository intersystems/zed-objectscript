//! Class comparison test plan: `documentation/class-comparison-test-plan.md`.
//!
//! Each test name starts with its plan ID. Tests assert the behavior described in
//! `documentation/class-comparison-expected-behavior.md`; tests for known gaps are
//! `#[ignore]`d with the reason so `cargo test -- --ignored` lists them.

use objectscript_core::parse_structures::{
    Cardinality, CodeMode, ForeignKeyAction, IndexType, InheritanceDirection, Language,
    MethodType, ReturnType, TriggerFire, TriggerForEach, TypeName,
};
use objectscript_core::parse_structures::FileType;
use objectscript_core::workspace::ProjectState;
use objectscript_core::workspace_diff::{
    ClassComparison, ClassDiff, MethodDiff, ValueChange, compare_class,
};
use tower_lsp::lsp_types::Url;

const CLASS: &str = "Demo.T";

fn project(sources: &[(&str, &str)]) -> ProjectState {
    let project = ProjectState::new();
    for (class_name, source) in sources {
        project.handle_document_opened(
            Url::parse(&format!("file:///workspace/{class_name}.cls")).unwrap(),
            source.to_string(),
            FileType::Cls,
            1,
        );
    }
    project
}

/// Wraps member definitions in `Class Demo.T Extends %Persistent`.
fn cls(body: &str) -> String {
    format!("Class {CLASS} Extends %Persistent\n{{\n\n{body}\n\n}}\n")
}

fn compare_sources(before: &str, after: &str) -> ClassComparison {
    let baseline = project(&[(CLASS, before)]);
    let target = project(&[(CLASS, after)]);
    let baseline = baseline.data.read();
    let target = target.data.read();
    compare_class(&baseline, &target, CLASS)
}

fn class_diff(before: &str, after: &str) -> ClassDiff {
    match compare_sources(before, after) {
        ClassComparison::Changed(diff) => diff,
        other => panic!("expected Changed, got {other:?}"),
    }
}

/// Compares two class bodies built with `cls`.
fn diff(before: &str, after: &str) -> ClassDiff {
    class_diff(&cls(before), &cls(after))
}

fn change<T>(before: T, after: T) -> Option<ValueChange<T>> {
    Some(ValueChange { before, after })
}

fn ty(ret_type: ReturnType) -> TypeName {
    TypeName {
        ret_type,
        parameters: Vec::new(),
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

/// No class-level field and no member collection changed.
fn assert_no_semantic_change(diff: &ClassDiff) {
    assert!(
        !diff.has_semantic_changes(),
        "expected no semantic change, got {diff:#?}"
    );
    assert!(diff.unmodeled_source_change);
}

macro_rules! assert_added_removed {
    ($field:ident, $before:expr, $after:expr, $name:expr) => {{
        let added = diff($before, $after);
        assert_eq!(added.$field.added, vec![$name.to_string()], "{added:#?}");
        assert!(added.$field.removed.is_empty() && added.$field.changed.is_empty());
        assert!(!added.unmodeled_source_change);

        let removed = diff($after, $before);
        assert_eq!(removed.$field.removed, vec![$name.to_string()], "{removed:#?}");
        assert!(removed.$field.added.is_empty() && removed.$field.changed.is_empty());
    }};
}

/// Returns the single changed member diff and asserts nothing else in the class changed.
macro_rules! only_changed {
    ($diff:expr, $field:ident, $name:expr) => {{
        let diff = &$diff;
        assert!(diff.$field.added.is_empty(), "{diff:#?}");
        assert!(diff.$field.removed.is_empty(), "{diff:#?}");
        assert_eq!(
            diff.$field.changed.keys().cloned().collect::<Vec<_>>(),
            vec![$name.to_string()],
            "{diff:#?}"
        );
        assert!(!diff.unmodeled_source_change);
        diff.$field.changed[$name].clone()
    }};
}

fn method_diff(before: &str, after: &str) -> MethodDiff {
    only_changed!(diff(before, after), methods, "Run")
}

/// A method diff that changes only the method source.
fn assert_source_only(method: &MethodDiff) {
    assert_eq!(
        method,
        &MethodDiff {
            source_changed: true,
            variables: method.variables.clone(),
            ..Default::default()
        },
        "expected only a source change"
    );
}

// ---------------------------------------------------------------------------
// C: class-level results and fields
// ---------------------------------------------------------------------------

#[test]
fn c01_identical_source_is_unchanged() {
    let source = cls("Property Name As %String;");
    assert_eq!(
        compare_sources(&source, &source),
        ClassComparison::Unchanged {
            class_name: CLASS.to_string()
        }
    );
}

#[test]
fn c02_class_only_in_target_is_added() {
    let baseline = project(&[]);
    let target = project(&[(CLASS, &cls(""))]);
    assert_eq!(
        compare_class(&baseline.data.read(), &target.data.read(), CLASS),
        ClassComparison::Added {
            class_name: CLASS.to_string()
        }
    );
}

#[test]
fn c03_class_only_in_baseline_is_removed() {
    let baseline = project(&[(CLASS, &cls(""))]);
    let target = project(&[]);
    assert_eq!(
        compare_class(&baseline.data.read(), &target.data.read(), CLASS),
        ClassComparison::Removed {
            class_name: CLASS.to_string()
        }
    );
}

#[test]
fn c04_final_keyword() {
    let diff = class_diff(
        "Class Demo.T Extends %Persistent\n{\n}\n",
        "Class Demo.T Extends %Persistent [ Final ]\n{\n}\n",
    );
    assert_eq!(diff.final_keyword, change(false, true));
    assert!(!diff.unmodeled_source_change);
}

#[test]
fn c05_procedure_block_keyword() {
    let diff = class_diff(
        "Class Demo.T Extends %Persistent\n{\n}\n",
        "Class Demo.T Extends %Persistent [ Not ProcedureBlock ]\n{\n}\n",
    );
    assert_eq!(diff.procedure_block, change(true, false));
}

#[test]
fn c06_language_keyword() {
    let diff = class_diff(
        "Class Demo.T Extends %Persistent\n{\n}\n",
        "Class Demo.T Extends %Persistent [ Language = tsql ]\n{\n}\n",
    );
    assert_eq!(diff.language, change(Language::Objectscript, Language::TSql));
}

#[test]
fn c07_inheritance_keyword() {
    let diff = class_diff(
        "Class Demo.T Extends (%Persistent, %Populate)\n{\n}\n",
        "Class Demo.T Extends (%Persistent, %Populate) [ Inheritance = right ]\n{\n}\n",
    );
    assert_eq!(
        diff.inheritance_direction,
        change(InheritanceDirection::Left, InheritanceDirection::Right)
    );
}

#[test]
fn c08_superclass_change() {
    let diff = class_diff(
        "Class Demo.T Extends %Persistent\n{\n}\n",
        "Class Demo.T Extends %RegisteredObject\n{\n}\n",
    );
    assert_eq!(
        diff.inherited_classes,
        change(names(&["%Persistent"]), names(&["%RegisteredObject"]))
    );
}

#[test]
fn c09_superclass_order_is_significant() {
    let diff = class_diff(
        "Class Demo.T Extends (%Persistent, %Populate)\n{\n}\n",
        "Class Demo.T Extends (%Populate, %Persistent)\n{\n}\n",
    );
    assert_eq!(
        diff.inherited_classes,
        change(
            names(&["%Persistent", "%Populate"]),
            names(&["%Populate", "%Persistent"])
        )
    );
}

#[test]
fn c10_import_change() {
    let diff = class_diff(
        "Import Demo.A\n\nClass Demo.T Extends %Persistent\n{\n}\n",
        "Import Demo.B\n\nClass Demo.T Extends %Persistent\n{\n}\n",
    );
    assert_eq!(diff.imports, change(names(&["Demo.A"]), names(&["Demo.B"])));
}

#[test]
fn c11_import_order_is_not_significant() {
    let diff = class_diff(
        "Import (Demo.A, Demo.B)\n\nClass Demo.T Extends %Persistent\n{\n}\n",
        "Import (Demo.B, Demo.A)\n\nClass Demo.T Extends %Persistent\n{\n}\n",
    );
    assert_no_semantic_change(&diff);
}

#[test]
fn c12_untracked_class_keyword_is_reported_as_unmodeled() {
    let diff = class_diff(
        "Class Demo.T Extends %Persistent\n{\n}\n",
        "Class Demo.T Extends %Persistent [ Abstract ]\n{\n}\n",
    );
    assert_no_semantic_change(&diff);
}

#[test]
fn c13_comment_only_change_is_reported_as_unmodeled() {
    let diff = diff(
        "Property Name As %String;",
        "/// The name.\nProperty Name As %String;",
    );
    assert_no_semantic_change(&diff);
}

#[test]
fn c14_keyword_case_is_not_significant() {
    let diff = class_diff(
        "Class Demo.T Extends %Persistent [ Final, Language = TSQL ]\n{\n}\n",
        "Class Demo.T Extends %Persistent [ final, language = tsql ]\n{\n}\n",
    );
    assert_no_semantic_change(&diff);
}

#[test]
#[ignore = "gap: same as q06, query argument Ranges shift when members move"]
fn c15_reordering_members_is_not_a_member_change() {
    let diff = diff(
        "Property A As %String;\n\nParameter P = 1;\n\nMethod Run()\n{\n    quit\n}\n\n\
         Index AIdx On A;\n\nQuery Q(x As %String) As %SQLQuery\n{\nSELECT ID FROM Demo.T\n}",
        "Query Q(x As %String) As %SQLQuery\n{\nSELECT ID FROM Demo.T\n}\n\nIndex AIdx On A;\n\n\
         Method Run()\n{\n    quit\n}\n\nParameter P = 1;\n\nProperty A As %String;",
    );
    assert_no_semantic_change(&diff);
}

// ---------------------------------------------------------------------------
// M: methods
// ---------------------------------------------------------------------------

const RUN: &str = "Method Run(a As %String) As %Status\n{\n    quit 1\n}";

#[test]
fn m01_method_added_and_removed() {
    assert_added_removed!(methods, "", RUN, "Run");
}

#[test]
fn m02_return_type() {
    let method = method_diff(RUN, &RUN.replace("As %Status", "As %Integer"));
    assert_eq!(
        method.return_type,
        change(Some(ty(ReturnType::Status)), Some(ty(ReturnType::Integer)))
    );
    assert!(method.source_changed);
}

#[test]
fn m03_argument_added() {
    let method = method_diff(RUN, &RUN.replace("(a As %String)", "(a As %String, b)"));
    assert_eq!(method.arguments.added, names(&["b"]));
}

#[test]
fn m04_argument_removed() {
    let method = method_diff(RUN, &RUN.replace("(a As %String)", "()"));
    assert_eq!(method.arguments.removed, names(&["a"]));
}

#[test]
fn m05_argument_type() {
    let method = method_diff(RUN, &RUN.replace("(a As %String)", "(a As %Integer)"));
    let argument = &method.arguments.changed["a"];
    assert_eq!(argument.before.return_type, Some(ty(ReturnType::String)));
    assert_eq!(argument.after.return_type, Some(ty(ReturnType::Integer)));
}

#[test]
fn m06_argument_default_value() {
    let method = method_diff(
        &RUN.replace("(a As %String)", "(a As %String = 1)"),
        &RUN.replace("(a As %String)", "(a As %String = 2)"),
    );
    let argument = &method.arguments.changed["a"];
    assert_eq!(argument.before.default_value.as_deref(), Some("1"));
    assert_eq!(argument.after.default_value.as_deref(), Some("2"));
}

#[test]
fn m07_argument_byref_and_output() {
    let byref = method_diff(RUN, &RUN.replace("(a As", "(ByRef a As"));
    let argument = &byref.arguments.changed["a"];
    assert!(!argument.before.byref && argument.after.byref);

    let output = method_diff(RUN, &RUN.replace("(a As", "(Output a As"));
    let argument = &output.arguments.changed["a"];
    assert!(!argument.before.output && argument.after.output);
}

#[test]
#[ignore = "gap: arguments are keyed by name, so a reordered signature is only a source change"]
fn m08_argument_order() {
    let method = method_diff(
        "Method Run(a, b)\n{\n    quit\n}",
        "Method Run(b, a)\n{\n    quit\n}",
    );
    assert!(
        method != MethodDiff {
            source_changed: true,
            ..Default::default()
        },
        "argument order is an API change, not just a source change"
    );
}

#[test]
fn m09_public_list() {
    let method = method_diff(
        "Method Run() [ PublicList = (x, y) ]\n{\n    quit\n}",
        "Method Run() [ PublicList = (x, z) ]\n{\n    quit\n}",
    );
    assert_eq!(
        method.public_variables_declared,
        change(names(&["x", "y"]), names(&["x", "z"]))
    );
}

#[test]
fn m10_private_keyword() {
    let method = method_diff(RUN, &RUN.replace("As %Status", "As %Status [ Private ]"));
    assert_eq!(method.is_public, change(true, false));
}

#[test]
fn m11_final_keyword() {
    let method = method_diff(RUN, &RUN.replace("As %Status", "As %Status [ Final ]"));
    assert_eq!(method.final_keyword, change(None, Some(true)));
}

#[test]
fn m12_language_keyword() {
    let method = method_diff(RUN, &RUN.replace("As %Status", "As %Status [ Language = tsql ]"));
    assert_eq!(method.language, change(None, Some(Language::TSql)));
}

#[test]
fn m13_codemode_keyword() {
    let method = method_diff(
        RUN,
        &RUN.replace("As %Status", "As %Status [ CodeMode = objectgenerator ]"),
    );
    assert_eq!(
        method.code_mode,
        change(CodeMode::Code, CodeMode::ObjectGenerator)
    );
}

#[test]
fn m14_procedure_block_keyword() {
    let method = method_diff(
        RUN,
        &RUN.replace("As %Status", "As %Status [ ProcedureBlock = 0 ]"),
    );
    assert_eq!(method.procedure_block, change(None, Some(false)));
}

#[test]
fn m15_instance_to_class_method() {
    let method = method_diff(RUN, &format!("Class{RUN}"));
    assert_eq!(
        method.method_type,
        change(MethodType::InstanceMethod, MethodType::ClassMethod)
    );
}

#[test]
fn m16_body_change_is_source_only() {
    let method = method_diff(RUN, &RUN.replace("quit 1", "quit 2"));
    assert_source_only(&method);
}

#[test]
fn m17_untracked_keyword_is_source_only() {
    let method = method_diff(RUN, &RUN.replace("As %Status", "As %Status [ SqlProc ]"));
    assert_source_only(&method);
}

#[test]
fn m18_variable_changes() {
    let method = method_diff(
        "Method Run()\n{\n    set oldVar = 1\n}",
        "Method Run()\n{\n    set newVar = 1\n}",
    );
    assert!(method.variables.removed.iter().any(|v| v.name == "oldVar"));
    assert!(method.variables.added.iter().any(|v| v.name == "newVar"));
    assert!(method.source_changed);
}

#[test]
fn m19_moving_method_is_not_a_change() {
    let diff = diff(RUN, &format!("\n\n\n{RUN}"));
    assert_no_semantic_change(&diff);
}

// ---------------------------------------------------------------------------
// P: properties
// ---------------------------------------------------------------------------

#[test]
fn p01_property_added_and_removed() {
    assert_added_removed!(properties, "", "Property Name As %String;", "Name");
}

#[test]
fn p02_return_type() {
    let property = only_changed!(
        diff("Property Name As %String;", "Property Name As %Integer;"),
        properties,
        "Name"
    );
    assert_eq!(
        property.return_type,
        change(Some(ty(ReturnType::String)), Some(ty(ReturnType::Integer)))
    );
}

#[test]
#[ignore = "gap: type parameter values (MAXLEN = 50) are dropped; only names are kept"]
fn p03_type_parameter_value() {
    let property = only_changed!(
        diff(
            "Property Name As %String(MAXLEN = 50);",
            "Property Name As %String(MAXLEN = 100);"
        ),
        properties,
        "Name"
    );
    assert!(property.return_type.is_some());
}

#[test]
fn p04_type_parameter_added() {
    let property = only_changed!(
        diff(
            "Property Name As %String;",
            "Property Name As %String(MAXLEN = 50);"
        ),
        properties,
        "Name"
    );
    assert!(property.return_type.is_some());
}

#[test]
#[ignore = "bug: get_tracked_keywords inverts Required (bare keyword sets false)"]
fn p05_required_keyword() {
    let property = only_changed!(
        diff(
            "Property Name As %String;",
            "Property Name As %String [ Required ];"
        ),
        properties,
        "Name"
    );
    assert_eq!(property.required, change(false, true));
}

#[test]
fn p06_private_keyword() {
    let property = only_changed!(
        diff(
            "Property Name As %String;",
            "Property Name As %String [ Private ];"
        ),
        properties,
        "Name"
    );
    assert_eq!(property.is_public, change(true, false));
}

#[test]
fn p07_final_keyword() {
    let property = only_changed!(
        diff(
            "Property Name As %String;",
            "Property Name As %String [ Final ];"
        ),
        properties,
        "Name"
    );
    assert_eq!(property.final_keyword, change(None, Some(true)));
}

#[test]
#[ignore = "bug: get_tracked_keywords inverts MultiDimensional (bare keyword sets false)"]
fn p08_multidimensional_keyword() {
    let property = only_changed!(
        diff(
            "Property Name As %String;",
            "Property Name As %String [ MultiDimensional ];"
        ),
        properties,
        "Name"
    );
    assert_eq!(property.multidimensional, change(false, true));
}

#[test]
#[ignore = "gap: untracked property keywords are only visible as a class-level unmodeled change"]
fn p09_untracked_keyword_is_reported_on_member() {
    let _ = only_changed!(
        diff(
            "Property Name As %String;",
            "Property Name As %String [ InitialExpression = \"x\" ];"
        ),
        properties,
        "Name"
    );
}

#[test]
fn p10_moving_property_is_not_a_change() {
    let diff = diff(
        "Property Name As %String(MAXLEN = 50) [ Required ];",
        "\n\n\nProperty Name As %String(MAXLEN = 50) [ Required ];",
    );
    assert_no_semantic_change(&diff);
}

// ---------------------------------------------------------------------------
// PA: parameters
// ---------------------------------------------------------------------------

#[test]
fn pa01_parameter_added_and_removed() {
    assert_added_removed!(parameters, "", "Parameter VERSION = 1;", "VERSION");
}

#[test]
#[ignore = "gap: ParameterSnapshot does not include default_value"]
fn pa02_default_value() {
    let _ = only_changed!(
        diff("Parameter VERSION = 1;", "Parameter VERSION = 2;"),
        parameters,
        "VERSION"
    );
}

#[test]
#[ignore = "bug: build_parameter_struct matches \"return_type\", grammar emits \"parameter_type\""]
fn pa03_return_type() {
    let parameter = only_changed!(
        diff(
            "Parameter VERSION As %String = 1;",
            "Parameter VERSION As %Integer = 1;"
        ),
        parameters,
        "VERSION"
    );
    assert_eq!(
        parameter.return_type,
        change(Some(ty(ReturnType::String)), Some(ty(ReturnType::Integer)))
    );
}

#[test]
fn pa04_final_keyword() {
    let parameter = only_changed!(
        diff("Parameter VERSION = 1;", "Parameter VERSION [ Final ] = 1;"),
        parameters,
        "VERSION"
    );
    assert_eq!(parameter.final_keyword, change(None, Some(true)));
}

#[test]
#[ignore = "gap: untracked parameter keywords are only visible as a class-level unmodeled change"]
fn pa05_untracked_keyword_is_reported_on_member() {
    let _ = only_changed!(
        diff("Parameter VERSION = 1;", "Parameter VERSION [ Internal ] = 1;"),
        parameters,
        "VERSION"
    );
}

// ---------------------------------------------------------------------------
// R: relationships
// ---------------------------------------------------------------------------

const REL: &str = "Relationship Items As Demo.Item [ Cardinality = many, Inverse = Owner ];";

#[test]
fn r01_relationship_added_and_removed() {
    assert_added_removed!(relationships, "", REL, "Items");
}

#[test]
fn r02_cardinality() {
    let relationship = only_changed!(
        diff(REL, &REL.replace("many", "children")),
        relationships,
        "Items"
    );
    assert_eq!(relationship.before.cardinality, Cardinality::Many);
    assert_eq!(relationship.after.cardinality, Cardinality::Children);
}

#[test]
fn r03_inverse() {
    let relationship = only_changed!(
        diff(REL, &REL.replace("Owner", "Parent")),
        relationships,
        "Items"
    );
    assert_ne!(relationship.before.inverse, relationship.after.inverse);
}

#[test]
fn r04_return_type() {
    let relationship = only_changed!(
        diff(REL, &REL.replace("Demo.Item", "Demo.Other")),
        relationships,
        "Items"
    );
    assert_ne!(relationship.before.return_type, relationship.after.return_type);
}

#[test]
#[ignore = "bug: get_tracked_keywords inverts Required (bare keyword sets false)"]
fn r05_required_private_final_keywords() {
    let relationship = only_changed!(
        diff(REL, &REL.replace(" ];", ", Required, Private, Final ];")),
        relationships,
        "Items"
    );
    assert!(!relationship.before.required && relationship.after.required);
    assert!(relationship.before.is_public && !relationship.after.is_public);
    assert_eq!(
        (relationship.before.is_final, relationship.after.is_final),
        (None, Some(true))
    );
}

#[test]
fn r06_keyword_value_case_is_not_significant() {
    let diff = diff(REL, &REL.replace("many", "Many"));
    assert_no_semantic_change(&diff);
}

// ---------------------------------------------------------------------------
// FK: foreign keys
// ---------------------------------------------------------------------------

const FK: &str = "ForeignKey OwnerFK(Name) References Demo.Owner(NameIdx);";

#[test]
fn fk01_foreign_key_added_and_removed() {
    assert_added_removed!(foreign_keys, "", FK, "OwnerFK");
}

#[test]
fn fk02_properties_constrained() {
    let key = only_changed!(
        diff(FK, &FK.replace("(Name)", "(Name, Code)")),
        foreign_keys,
        "OwnerFK"
    );
    assert_eq!(key.before.properties_constrained, names(&["Name"]));
    assert_eq!(key.after.properties_constrained, names(&["Name", "Code"]));
}

#[test]
fn fk03_referenced_class() {
    let key = only_changed!(
        diff(FK, &FK.replace("Demo.Owner", "Demo.Other")),
        foreign_keys,
        "OwnerFK"
    );
    assert_eq!(key.before.referenced_class, "Demo.Owner");
    assert_eq!(key.after.referenced_class, "Demo.Other");
}

#[test]
fn fk04_referenced_index() {
    let key = only_changed!(
        diff(FK, &FK.replace("(NameIdx)", "(CodeIdx)")),
        foreign_keys,
        "OwnerFK"
    );
    assert_eq!(key.before.referenced_index.as_deref(), Some("NameIdx"));
    assert_eq!(key.after.referenced_index.as_deref(), Some("CodeIdx"));
}

#[test]
fn fk05_on_delete_and_on_update_keywords() {
    let key = only_changed!(
        diff(
            FK,
            &FK.replace(";", " [ OnDelete = cascade, OnUpdate = setnull ];")
        ),
        foreign_keys,
        "OwnerFK"
    );
    assert_eq!(
        (key.before.on_delete, key.after.on_delete),
        (ForeignKeyAction::NoAction, ForeignKeyAction::Cascade)
    );
    assert_eq!(
        (key.before.on_update, key.after.on_update),
        (ForeignKeyAction::NoAction, ForeignKeyAction::SetNull)
    );
}

#[test]
#[ignore = "gap: untracked foreign key keywords are only visible as a class-level unmodeled change"]
fn fk06_untracked_keyword_is_reported_on_member() {
    let _ = only_changed!(
        diff(FK, &FK.replace(";", " [ Internal ];")),
        foreign_keys,
        "OwnerFK"
    );
}

// ---------------------------------------------------------------------------
// Q: queries
// ---------------------------------------------------------------------------

const QUERY: &str =
    "Query ByName(name As %String) As %SQLQuery\n{\nSELECT ID FROM Demo.T WHERE Name = :name\n}";

#[test]
fn q01_query_added_and_removed() {
    assert_added_removed!(queries, "", QUERY, "ByName");
}

#[test]
fn q02_arguments() {
    let query = only_changed!(
        diff(QUERY, &QUERY.replace("name As %String", "name As %Integer")),
        queries,
        "ByName"
    );
    assert_ne!(
        query.before.arguments["name"].0,
        query.after.arguments["name"].0
    );
}

#[test]
fn q03_return_type() {
    let query = only_changed!(
        diff(QUERY, &QUERY.replace("%SQLQuery", "%Query")),
        queries,
        "ByName"
    );
    assert_ne!(query.before.return_type, query.after.return_type);
}

#[test]
#[ignore = "gap: Query has no body content field"]
fn q04_body_content() {
    let _ = only_changed!(
        diff(QUERY, &QUERY.replace("SELECT ID", "SELECT ID, Name")),
        queries,
        "ByName"
    );
}

#[test]
fn q05_keywords() {
    let query = only_changed!(
        diff(
            QUERY,
            &QUERY.replace("%SQLQuery", "%SQLQuery [ Final, Private, Requires = \"R:U\" ]")
        ),
        queries,
        "ByName"
    );
    assert_eq!((query.before.is_final, query.after.is_final), (None, Some(true)));
    assert!(query.before.is_public && !query.after.is_public);
    assert!(query.before.required_privileges.is_empty());
    assert!(!query.after.required_privileges.is_empty());
}

#[test]
#[ignore = "gap: Query snapshots include argument source Ranges, so moving a query reports a change"]
fn q06_moving_query_is_not_a_change() {
    let diff = diff(QUERY, &format!("\n\n\n{QUERY}"));
    assert_no_semantic_change(&diff);
}

// ---------------------------------------------------------------------------
// I: indexes
// ---------------------------------------------------------------------------

#[test]
fn i01_index_added_and_removed() {
    assert_added_removed!(indices, "", "Index NameIdx On Name;", "NameIdx");
}

#[test]
fn i02_properties() {
    let index = only_changed!(
        diff("Index NameIdx On Name;", "Index NameIdx On (Name, Code);"),
        indices,
        "NameIdx"
    );
    assert_eq!(index.before.properties.len(), 1);
    assert_eq!(index.after.properties.len(), 2);
}

#[test]
fn i03_index_type() {
    let index = only_changed!(
        diff(
            "Index NameIdx On Name;",
            "Index NameIdx On Name [ Type = bitmap ];"
        ),
        indices,
        "NameIdx"
    );
    assert_eq!(
        (index.before.index_type, index.after.index_type),
        (IndexType::Index, IndexType::Bitmap)
    );
}

#[test]
fn i04_property_collation() {
    let index = only_changed!(
        diff(
            "Index NameIdx On Name As SQLUPPER;",
            "Index NameIdx On Name As EXACT;"
        ),
        indices,
        "NameIdx"
    );
    assert_ne!(
        index.before.properties[0].return_type,
        index.after.properties[0].return_type
    );
}

#[test]
#[ignore = "gap: Unique/IdKey/PrimaryKey are not modeled and not reported on the member"]
fn i05_untracked_keyword_is_reported_on_member() {
    let _ = only_changed!(
        diff("Index NameIdx On Name;", "Index NameIdx On Name [ Unique ];"),
        indices,
        "NameIdx"
    );
}

// ---------------------------------------------------------------------------
// T: triggers
// ---------------------------------------------------------------------------

const TRIGGER: &str = "Trigger Log [ Event = INSERT ]\n{\n    set ^Log = 1\n}";

#[test]
fn t01_trigger_added_and_removed() {
    assert_added_removed!(triggers, "", TRIGGER, "Log");
}

#[test]
fn t02_event() {
    let trigger = only_changed!(
        diff(TRIGGER, &TRIGGER.replace("INSERT", "INSERT/UPDATE")),
        triggers,
        "Log"
    );
    assert!(!trigger.before.update && trigger.after.update);
}

#[test]
fn t03_time() {
    let trigger = only_changed!(
        diff(TRIGGER, &TRIGGER.replace(" ]", ", Time = AFTER ]")),
        triggers,
        "Log"
    );
    assert_eq!(
        (trigger.before.time, trigger.after.time),
        (TriggerFire::BEFORE, TriggerFire::AFTER)
    );
}

#[test]
fn t04_foreach() {
    let trigger = only_changed!(
        diff(TRIGGER, &TRIGGER.replace(" ]", ", Foreach = row/object ]")),
        triggers,
        "Log"
    );
    assert_eq!(
        (trigger.before.for_each, trigger.after.for_each),
        (TriggerForEach::Row, TriggerForEach::RowObject)
    );
}

#[test]
fn t05_final_language_codemode() {
    let trigger = only_changed!(
        diff(
            TRIGGER,
            &TRIGGER.replace(" ]", ", Final, Language = tsql, CodeMode = objectgenerator ]")
        ),
        triggers,
        "Log"
    );
    assert_eq!(
        (trigger.before.is_final, trigger.after.is_final),
        (None, Some(true))
    );
    assert_eq!(trigger.after.language, Language::TSql);
    assert_eq!(trigger.after.code_mode, CodeMode::ObjectGenerator);
}

// ---------------------------------------------------------------------------
// X: XData
// ---------------------------------------------------------------------------

const XDATA: &str = "XData Config [ MimeType = application/json ]\n{\n{\"a\": 1}\n}";

#[test]
fn x01_xdata_added_and_removed() {
    assert_added_removed!(xdata, "", XDATA, "Config");
}

#[test]
fn x02_mimetype() {
    let xdata = only_changed!(
        diff(XDATA, &XDATA.replace("application/json", "text/markdown")),
        xdata,
        "Config"
    );
    assert_eq!(
        (xdata.before.language, xdata.after.language),
        (Language::Json, Language::Markdown)
    );
}

#[test]
fn x03_body_change_is_reported_as_unmodeled() {
    let diff = diff(XDATA, &XDATA.replace("1", "2"));
    assert_no_semantic_change(&diff);
}

// ---------------------------------------------------------------------------
// PR: projections
// ---------------------------------------------------------------------------

const PROJECTION: &str = "Projection JavaProj As %Projection.Java;";

#[test]
fn pr01_projection_added_and_removed() {
    assert_added_removed!(projections, "", PROJECTION, "JavaProj");
}

#[test]
fn pr02_return_type() {
    let projection = only_changed!(
        diff(PROJECTION, &PROJECTION.replace("%Projection.Java", "%Projection.Monitor")),
        projections,
        "JavaProj"
    );
    assert_ne!(projection.before.return_type, projection.after.return_type);
}

#[test]
#[ignore = "gap: untracked projection keywords are only visible as a class-level unmodeled change"]
fn pr03_untracked_keyword_is_reported_on_member() {
    let _ = only_changed!(
        diff(PROJECTION, &PROJECTION.replace(";", " [ Internal ];")),
        projections,
        "JavaProj"
    );
}

// ---------------------------------------------------------------------------
// S: storage
// ---------------------------------------------------------------------------

const STORAGE: &str = "Storage Default\n{\n<Type>%Storage.Persistent</Type>\n}";

#[test]
fn s01_storage_added_and_removed() {
    assert_added_removed!(storage, "", STORAGE, "Default");
}

#[test]
fn s02_body_change_is_reported_as_unmodeled() {
    let diff = diff(STORAGE, &STORAGE.replace("Persistent", "SQL"));
    assert_no_semantic_change(&diff);
}
