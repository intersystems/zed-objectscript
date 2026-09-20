# Granular Class Comparison

`objectscript_core::workspace_diff` compares classes from independently indexed
workspaces without comparing workspace-local IDs.

This is intended for the IRIS upgrade workflow:

1. Index SYS version 1 and the customer workspace together.
2. Compute the transitive SYS dependency-name set from version 1.
3. Index SYS version 2 independently.
4. Compare only those dependent class names between the two projects.

## API

```rust
use objectscript_core::workspace_diff::{
    ClassComparison,
    compare_class,
    compare_classes_parallel,
};
```

Compare one class:

```rust
let comparison = compare_class(
    &baseline_project_data,
    &target_project_data,
    "%Library.Example",
);
```

Compare a dependency-name set in parallel:

```rust
let comparisons = compare_classes_parallel(
    &dependent_sys_class_names,
    &baseline_project_data,
    &target_project_data,
);
```

Both projects are read-only during comparison. The outer operation parallelizes
by class; member comparison inside one class remains serial to avoid nested
Rayon work.

## Fast path

The comparator first looks up each class's defining `Document`.

- Equal source strings return `ClassComparison::Unchanged` immediately.
- A missing target class returns `Removed`.
- A missing baseline class returns `Added`.
- Different source strings trigger semantic snapshot construction and granular
  comparison.

This avoids allocating snapshots for unchanged SYS classes.

## ID-independent snapshots

`ClassId`, `MethodRef`, `PropertyRef`, `ParameterRef`, `VariableRef`, and
`ScopeId` are local to one workspace and are never compared directly.

The comparator dereferences them and builds value snapshots keyed by class and
member names.

### Class fields

- direct imports, normalized by sorting;
- direct inherited class names, preserving parent order;
- inheritance direction;
- `ProcedureBlock`;
- `Language`;
- `Final`;
- own methods;
- own properties;
- own parameters.

Inherited effective-member maps are not compared as if they were declarations.
They can be derived after direct class changes are known.

### Method fields

- method kind;
- return type;
- visibility;
- `ProcedureBlock`;
- `Language`;
- `CodeMode`;
- `Final`;
- sorted `PublicList` names;
- variables;
- exact method declaration/body source.

The current method symbol range includes the method source represented by the
semantic model. `MethodDiff::source_changed` is deliberately named “source,”
not “body,” because a signature/keyword edit can also change this range.

### Variable fields

- name;
- argument type, when represented;
- public/private status;
- inferred `VariableDefType`.

Variables are compared as multisets. Workspace-local variable and scope IDs are
ignored, and repeated equivalent definitions retain occurrence counts.

### Property fields

- return type;
- required;
- visibility;
- final;
- multidimensional.

### Parameter fields

- return type;
- final.

## Result model

```rust
pub enum ClassComparison {
    Unchanged { class_name: String },
    Added { class_name: String },
    Removed { class_name: String },
    SnapshotUnavailable {
        class_name: String,
        baseline_available: bool,
        target_available: bool,
    },
    Changed(ClassDiff),
}
```

`ClassDiff` reports changed class fields and added, removed, or changed methods,
properties, and parameters. Each changed member contains field-level
`ValueChange { before, after }` values.

`unmodeled_source_change` is true when class source differs but none of the
currently modeled semantic fields differ. This preserves visibility into
unsupported UDL constructs, comments, or formatting changes.

## Performance characteristics

- Source equality is $O(n)$ in class source bytes and allocates nothing.
- Snapshots are built only for changed classes.
- Class comparisons run through Rayon.
- Maps in snapshots use `BTreeMap` for readable stable reports.
- Variable multiset comparison is quadratic in the number of definitions in one
  method. Methods normally contain few definitions; replace it with counted
  hash keys only if profiling identifies it as significant.
- Exact method source is cloned only for changed classes while their snapshots
  are compared.

The expensive operation remains workspace indexing. Comparison should be run
after both workspace indexes are complete.

## Current limitations

The report can only compare facts retained by the semantic model. It does not
yet fully model:

- ordered method argument signatures and every argument modifier/default;
- relationships;
- indexes;
- queries;
- triggers;
- XData;
- storage definitions;
- include and macro dependencies.

Exact class and method source-change reporting prevents these differences from
being silently treated as unchanged. Add typed snapshots as these constructs
enter the semantic model.

## Dependency closure is separate

The comparison module accepts class names; it does not decide which SYS classes
the customer depends on.

The dependency-closure implementation should:

1. start from all customer-owned methods in the baseline workspace;
2. traverse outgoing ordinary and oref call edges transitively;
3. add reached SYS class names;
4. recursively add direct superclass names;
5. pass the resulting names to `compare_classes_parallel`.

Use class names as cross-workspace identity. Never pass baseline IDs into the
target workspace.
