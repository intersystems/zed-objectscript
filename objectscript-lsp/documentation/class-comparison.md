# Granular Class Comparison

`objectscript_core::workspace_diff` compares classes from independently indexed
workspaces without comparing workspace-local IDs.

This is intended for the IRIS upgrade workflow:

1. Index SYS version 1 and the customer workspace together.
2. Compute the transitive SYS dependency-name set from version 1.
3. Index SYS version 2 independently.
4. Compare only those dependent class names between the two projects.

## Implementation status

### Implemented

- [x] ID-independent class snapshots.
- [x] ID-independent method, variable, property, and parameter snapshots.
- [x] Exact source-equality short-circuit for unchanged classes.
- [x] Added, removed, unchanged, unavailable, and granular changed results.
- [x] Parallel comparison across requested class names.
- [x] Parallel comparison across the union of all classes in two workspaces.
- [x] Field-level class and member changes.
- [x] Unresolved-versus-resolved direct inheritance changes, even when class
      source is identical.
- [x] Variable comparison as a multiset rather than by workspace-local IDs.
- [x] Reporting of source differences not represented by the current semantic
      model.
- [x] Unit tests for the unchanged fast path and granular class/member/variable
      changes.
- [x] Public export through `objectscript_core::workspace_diff`.

### Remaining integration work

- [x] Compute the transitive customer-to-SYS dependency-name closure from the
      baseline workspace.
- [x] Add direct superclass names recursively to that closure.
- [x] Invoke `compare_classes_parallel` from the development two-version
      workflow in `main`.
- [x] Write a deterministic human-readable comparison report containing direct
      and transitive dependency names, summary counts, and granular differences.
- [ ] Decide whether the text report is the final supported output boundary or
      add JSON, LSP, or MCP presentation.
- [ ] Add integration tests that independently index two fixture workspaces and
      compare a realistic dependency closure.
- [ ] Add explicit tests for added classes, removed classes, unavailable
      snapshots, multiple inheritance ordering, and repeated equivalent
      variable definitions.
- [ ] Benchmark comparison over the real dependent SYS class set and report the
      unchanged-source hit rate.
- [ ] Decide whether source-only changes such as comments and formatting should
      appear in the final user-facing report or be optionally suppressed.
- [ ] Add typed snapshots for relationships, indexes, queries, triggers, XData,
      storage definitions, and include/macro dependencies as those constructs
      enter the semantic model.
- [ ] Consider serializable result types after the output boundary is chosen.
      Do not add serialization solely for internal comparison.

The comparison engine and development file-report workflow are implemented.
Production LSP/MCP exposure and broader integration coverage remain.

## Development workflow integration

The development `main` now:

1. indexes the baseline SYS root;
2. marks those classes as SYS;
3. appends and indexes the customer root in the same `ProjectData`;
4. indexes the target SYS root independently;
5. computes direct and transitive SYS dependency names from the baseline;
6. compares those names between baseline and target in parallel;
7. writes a human-readable report file.

Default paths match the local upgrade workspace. Override them without editing
source:

```sh
OBJECTSCRIPT_BASELINE_SYS_ROOT=/path/to/old/sys \
OBJECTSCRIPT_TARGET_SYS_ROOT=/path/to/new/sys \
OBJECTSCRIPT_CUSTOMER_ROOT=/path/to/customer \
OBJECTSCRIPT_COMPARISON_OUTPUT=/path/to/report.txt \
OBJECTSCRIPT_INDEX_WORKERS=8 \
cargo run --release --bin objectscript-lsp
```

By default, the development workflow compares only customer SYS dependencies.
To compare every class in the baseline SYS root with every class in the target
SYS root, omit customer indexing and use:

```sh
OBJECTSCRIPT_COMPARE_ALL=true \
OBJECTSCRIPT_BASELINE_SYS_ROOT=/path/to/old/sys \
OBJECTSCRIPT_TARGET_SYS_ROOT=/path/to/new/sys \
OBJECTSCRIPT_COMPARISON_OUTPUT=/path/to/all-classes-report.txt \
OBJECTSCRIPT_INDEX_WORKERS=8 \
cargo run --release --bin objectscript-lsp
```

The report contains:

- sorted direct SYS dependency names;
- sorted transitive-only SYS dependency names;
- unchanged/changed/added/removed/unavailable counts;
- detailed `ClassComparison` debug output for every non-unchanged class.

The normal LSP transport still starts with `--server` in this development
executable.

## API

```rust
use objectscript_core::workspace_diff::{
    ClassComparison,
    compare_class,
    compare_classes_parallel,
    compare_workspaces_parallel,
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

Compare the union of all class names in two workspaces:

```rust
let comparisons = compare_workspaces_parallel(
    &baseline_project_data,
    &target_project_data,
);
```

This reports classes found in only the target as `Added`, classes found only in
the baseline as `Removed`, and compares classes present in both. Result order is
unspecified; sort by class name only at presentation boundaries that require
stable output.

Both projects are read-only during comparison. The outer operation parallelizes
by class; member comparison inside one class remains serial to avoid nested
Rayon work.

## Fast path

The comparator first looks up each class's defining `Document`.

- Equal source strings return `ClassComparison::Unchanged` immediately only
  when both workspaces also have the same unresolved-parent set.
- A missing target class returns `Removed`.
- A missing baseline class returns `Added`.
- Different source strings trigger semantic snapshot construction and granular
  comparison.

This means identical child source is still reported as changed when a parent is
missing in one workspace and resolves in the other.

This avoids allocating snapshots for unchanged SYS classes.

## ID-independent snapshots

`ClassId`, `MethodRef`, `PropertyRef`, `ParameterRef`, `VariableRef`, and
`ScopeId` are local to one workspace and are never compared directly.

The comparator dereferences them and builds value snapshots keyed by class and
member names.

### Class fields

- direct imports, normalized by sorting;
- direct inherited class names, preserving parent order;
- unresolved direct inherited class names;
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

## Dependency closure

The comparison module accepts class names; it does not decide which SYS classes
the customer depends on.

`ProjectData::get_sys_dependencies` computes that selection separately. It:

1. Starts from all customer-owned methods in the baseline workspace.
2. Traverses outgoing ordinary and oref call edges transitively.
3. Records SYS classes directly called or inherited by customer classes.
4. Records additional SYS classes reached through SYS-to-SYS calls.
5. Recursively adds direct SYS superclass names.
6. Returns disjoint direct and transitive class-name sets.

The upgrade workflow should combine the returned names and pass them to
`compare_classes_parallel`.

Use class names as cross-workspace identity. Never pass baseline IDs into the
target workspace.
