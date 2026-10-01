# objectscript-core

The in-memory representation of an InterSystems ObjectScript workspace. `objectscript-core`
parses `.cls`, `.mac`, `.inc`, `.rtn`, `.int`, and `.xml` sources with `tree-sitter`, builds a
semantic model of classes, routines, members, and variables, and keeps the indexes needed for
navigation, diagnostics, refactoring, and dependency analysis.

It has no LSP transport of its own. [`objectscript-lsp`](../../README.md) wraps it as a language
server, but it can also be used directly (for example, to compare two workspaces offline).

## Concepts

- **`ProjectState`** (`workspace.rs`): concurrency wrapper for one workspace. Holds the root path,
  a lock-protected `ProjectData`, and shared Tree-sitter parsers. Entry point for opening and
  updating documents (`handle_document_opened`, `update_document`) and refactors.
- **`ProjectData`** (`workspace.rs`): the workspace "database": documents, configuration,
  semantic models, symbol indexes, the dependency graph, the override index, and diagnostics.
  Also answers definition/implementation queries (`get_method_definition`,
  `get_variable_definition`, `get_class_implementations`, and similar).
- **`GlobalSemanticModel`** (`global_semantic.rs`): public, workspace-wide symbols (classes,
  methods, properties, parameters, and every other class member type), keyed by stable refs
  such as `ClassId`, `MethodRef`, and `PropertyRef`.
- **`LocalSemanticModel`** + **`ScopeTree`** (`local_semantic.rs`, `scope_tree.rs`): private,
  per-class symbols and ProcedureBlock-aware lexical scopes.
- **`DependencyGraph`** (`dependency_tracker.rs`): caller → callee edges between methods, used to
  find every path into a scope (e.g. resolving public variables).
- **`OverrideIndex`** (`override_index.rs`): inherited/overridden members per class, built
  parent-before-child.
- **Class comparison** (`workspace_diff.rs`): ID-independent snapshots and diffs of classes and
  their members across two `ProjectData` instances.

## Module Layout

| Module | Responsibility |
| --- | --- |
| `workspace` | `ProjectState`, `ProjectData`, bulk indexing, incremental updates, queries |
| `workspace_diff` | `snapshot_class`, `compare_class`, `compare_classes_parallel`, `compare_workspaces_parallel` |
| `class` | Building a `Class` and its members from a parse tree |
| `method`, `variable` | Method bodies, variable definitions, unresolved calls |
| `property`, `parameter`, `relationship`, `foreignkey`, `query`, `index`, `trigger`, `xdata`, `projection`, `storage` | Per-member-type extraction |
| `parse_structures` | Shared data types (`ClassId`, `FileType`, `Method`, `Property`, ...) |
| `global_semantic`, `local_semantic`, `scope_tree`, `scope_structures` | Semantic models and scopes |
| `dependency_tracker`, `override_index` | Call graph, dependents, and override tables |
| `refactor` | Legacy dotted `DO`, `IF/Else`, and `FOR` rewrites |
| `document` | Parsed document state (content, tree, version, file type) |
| `config` | Workspace configuration |
| `common` | Tree-sitter helpers, cached queries, position/byte conversion |

## Indexing Model

There are two build paths:

1. **Bulk (cold) indexing** — `ProjectData::begin_bulk_index()` returns a `BulkWorkspaceIndex`.
   Callers `register` every pre-parsed `BulkIndexDocument` (assigning `ClassId`s and detecting
   duplicate documents/classes), then call `finalize`:
   - **Parallel:** each class/routine document is built independently with `rayon`
     (`into_par_iter`): class members, method bodies, variables, and unresolved calls/orefs.
     Workers own their document and never touch `ProjectData`. `Class::build_class` additionally
     builds individual member definitions in parallel.
   - **Serial:** results are committed, then the full inheritance graph is built, then the
     override index, then ordinary call edges, then oref call resolution. Oref resolution walks
     incoming call edges, so it must follow the ordinary edges for the result to be
     deterministic.

   Reading and parsing the files themselves is the caller's job; `objectscript-lsp` does that in
   parallel too, with one parser set per rayon worker.

2. **Incremental updates** — `ProjectState::update_document` applies edits to a single document
   and rebuilds only the affected classes, members, inheritance, override entries, and call
   edges. See
   [`documentation/incremental-workspace-index-maintenance-spec.md`](../../documentation/incremental-workspace-index-maintenance-spec.md).

`ProjectData::mark_current_classes_as_sys()` can be called after indexing an IRIS SYS source
root and before indexing a customer workspace; `get_sys_dependencies()` then reports the
customer code's direct and transitive SYS dependencies.

## Usage

```rust
use objectscript_core::parse_structures::FileType;
use objectscript_core::workspace::ProjectState;
use objectscript_core::workspace_diff::compare_class;
use tower_lsp::lsp_types::Url;

let baseline = ProjectState::new();
baseline.handle_document_opened(
    Url::parse("file:///workspace/Demo.T.cls").unwrap(),
    source.to_string(),
    FileType::Cls,
    1,
);

let target = ProjectState::new();
// ... open documents in `target` ...

let comparison = compare_class(&baseline.data.read(), &target.data.read(), "Demo.T");
```

See [`tests/class_comparison.rs`](tests/class_comparison.rs) for more complete examples.

## Testing

```bash
cargo test -p objectscript-core
```

`tests/class_comparison.rs` covers class and member comparison. Tests for known gaps are
`#[ignore]`d with a reason; list them with `cargo test -p objectscript-core -- --ignored`.

## Benchmarks

```bash
# Incremental update_document benchmark (criterion)
cargo bench -p objectscript-core --features update-bench --bench update_document

# Incremental update benchmark as a standalone example
cargo run -p objectscript-core --release --features update-bench --example update_bench

# Cold workspace construction; pass a workspace path, or omit it to use a generated workspace
cargo run -p objectscript-core --release --example cold_workspace_bench -- /path/to/workspace
```

The `update-bench` feature enables instrumentation (e.g. `full_update_document_call_count`) that
is compiled out of normal builds. Inputs and options for the criterion bench are documented in
[`documentation/benchmarks/update-document.md`](../../documentation/benchmarks/update-document.md).

## Dependencies

- `tree-sitter 0.26.6` with `tree-sitter-objectscript`, `tree-sitter-objectscript-routine`,
  `tree-sitter-objectscript-playground` (`1.10.1`), and `tree-sitter-xml` (`0.7.0`)
- `rayon` for parallel indexing and comparison
- `petgraph` for the dependency graph
- `tower-lsp` for shared LSP types (`Url`, `Range`, `Diagnostic`)
