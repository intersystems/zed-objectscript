# ObjectScript LSP

Language Server & Language Server Protocol implementation for InterSystems ObjectScript using `tower-lsp` and `tree-sitter`.

We built this language server to provide editor-independent ObjectScript semantics for VS Code, Zed, Neovim, and other LSP clients without requiring a live InterSystems server connection.

## Current Features

- Workspace indexing for `.cls`, `.inc`, `.rtn`, `.mac`, `.int`, and `.xml`
- Parallel cold workspace indexing (see [Parallel Indexing](#parallel-indexing)); live documents are
  rebuilt incrementally.
- Multi-workspace support through LSP workspace folders, with deepest-parent routing per document
- Go-to-definition for ObjectScript variables, orefs, methods, properties, classes, parameters with ProcedureBlock-aware private/public resolution
- Go-to-implementation for inherited and overridden methods and classes
- Semantic model for every class member type: methods, properties, parameters, relationships,
  foreign keys, queries, indices, triggers, XData, projections, and storage
- Syntax diagnostics for tracked ObjectScript documents, plus duplicate-class diagnostics
- Mixed-language diagnostics for ObjectScript captured from XML `Implementation` blocks
- Refactor code actions for:
  - Legacy dotted `DO` rewrites
  - Legacy `IF/Else` rewrites
  - Legacy `FOR` rewrites
  - document-scoped and workspace-scoped edits
- Inheritance modeling and override index build
- Dependency modeling and dependencyGraph build (shows all paths to a given method)
- Granular, ID-independent class and member comparison between two workspaces
  (`objectscript_core::workspace_diff`)
- An MCP bridge binary (`objectscript-mcp`) that exposes the server to MCP clients such as Claude Code

## Architecture Summary

- One `ProjectState` is created per workspace folder
- Two-phase semantic build:
  - initial class/routine parse and symbol creation
  - inheritance, variables, and call extraction
- Public symbols live in `GlobalSemanticModel`
- Private symbols are tracked through `LocalSemanticModel` and `ScopeTree`
- XML documents are tracked for diagnostics, but they do not enter the class/routine semantic rebuild pipeline
- See [crates/objectscript-core/README.md](crates/objectscript-core/README.md) for the core
  library's module layout and public API.

### Parallel Indexing

Cold workspace indexing (run for each workspace folder on `initialized`) runs on Tokio's
blocking thread pool so the LSP event loop stays responsive, and uses `rayon` for data
parallelism. The rayon global pool is used, so the worker count defaults to the number of
logical CPUs (override with `RAYON_NUM_THREADS`).

1. **Discover** (serial): `get_paths` walks the workspace root and classifies supported files by
   extension.
2. **Read + parse** (parallel, per file): each rayon worker reads a file and parses it with its own
   `IndexParsers` instance (created once per worker via `map_init`), producing a
   `BulkIndexDocument`. Files that fail to read or parse are logged and skipped.
3. **Register** (serial): under the project write lock, `ProjectData::begin_bulk_index()`
   registers each document, assigns `ClassId`s, and records duplicate-class diagnostics.
4. **Build classes** (parallel, per document): `BulkWorkspaceIndex::finalize` builds each class's
   members and method bodies (variables, unresolved calls, unresolved oref calls) in parallel.
   Workers own their document and never observe `ProjectData`. Inside each class,
   `Class::build_class` also builds individual member definitions in parallel.
5. **Commit and link** (serial): results are committed into the shared semantic model, then the
   complete inheritance graph is built, followed by the override index (parent-before-child),
   ordinary method call edges, and finally oref call resolution. The order matters: oref
   resolution walks incoming call edges, so it must run after every ordinary edge exists.

All shared semantic/index mutation stays serial, so the result does not depend on scheduling.
Progress and per-phase timings are logged to stderr with an `[index]` prefix.

Workspace comparison is also parallel: `compare_classes_parallel` and
`compare_workspaces_parallel` diff classes across two `ProjectData` snapshots with `par_iter`.

## Workspace Layout

- `objectscript-lsp`: LSP transport layer in [src/main.rs](src/main.rs), [src/lsp.rs](src/lsp.rs), and [src/server.rs](src/server.rs)
  - [src/bin/objectscript-mcp.rs](src/bin/objectscript-mcp.rs): MCP bridge that spawns
    `objectscript-lsp` and exposes it as MCP tools
- `crates/objectscript-core`: parsing, semantic model, workspace state, refactors, dependency
  tracking, and class comparison ([README](crates/objectscript-core/README.md))
- `claude-code-objectscript-lsp/`: Claude Code LSP plugin ([README](claude-code-objectscript-lsp/README.md))
- `objectscript-tests/`: fixture corpus for inheritance, dependencies, navigation, and related regressions
- `documentation/`: feature docs, configuration, benchmarks, and design notes

## LSP Surface

- Standard requests:
  - `textDocument/definition`
  - `textDocument/implementation`
  - `textDocument/diagnostic`
  - `workspace/diagnostic`
  - `textDocument/codeAction` (`refactor.rewrite`)
  - `workspace/executeCommand`
- Notifications: `didOpen`, `didChange` (incremental sync), `didChangeConfiguration`,
  and `didChangeWatchedFiles`
- Execute commands:
  - `objectscript.refactorDocument`
  - `objectscript.refactorWorkspace`
  - `objectscript.refactorWorkspaceDottedDo` (legacy)
- Experimental capability: `objectscriptDependenciesProvider`

## MCP Tools

`objectscript-mcp` exposes: `objectscript_initialize_workspace`, `objectscript_diagnostics`,
`objectscript_workspace_diagnostics`, `objectscript_goto_definition`,
`objectscript_code_actions`, `objectscript_execute_command`, and `objectscript_lsp_status`.

## Configuration

Editor-specific configuration examples for Zed, Neovim, and VS Code are documented in [documentation/configuration.md](documentation/configuration.md).

## Build and Test

```bash
cargo build                 # builds objectscript-lsp and objectscript-mcp
cargo install --path .      # installs both binaries onto PATH
cargo test --workspace      # LSP tests + objectscript-core tests
```

Benchmarks live in `crates/objectscript-core`; see its [README](crates/objectscript-core/README.md#benchmarks)
and [documentation/benchmarks/update-document.md](documentation/benchmarks/update-document.md).

### Go-To Definition

Go-to definition works for classes, class methods, orefs, procedures, subroutines, instance methods, public local variables, private local variables, and global variables. 

For variables, the way the definition(s) is determined depends on the case: 

**CASE 1: Variable is defined the current scope.**
In this case, the variable definition in the given scope is returned.

**CASE 2: Variable is NOT defined the current scope.**

**CASE 2A: Variable is Private**
This means that the variable is undefined. No definition is returned.

**CASE 2B: Variable is Public**
In this case, the `DependencyGraph` is used to determine all possible paths to the current scope. For each node (scope) on the path, we check if the wanted variable is defined in that scope, and if so we track that location. All possible locations are returned.

More detail is in [documentation/features](documentation/features).

## Grammar Baseline

- `tree-sitter = 0.26.6`
- `tree-sitter-objectscript = 1.10.1`
- `tree-sitter-objectscript-routine = 1.10.1`
- `tree-sitter-objectscript-playground = 1.10.1`
- `tree-sitter-xml = 0.7.0`

## Roadmap

- Semantic diagnostics (undefined variables, unresolved symbols)
- Broader mixed-language support beyond XML `Implementation` blocks
- More lifecycle and incremental edit coverage
  (see [documentation/incremental-workspace-index-maintenance-spec.md](documentation/incremental-workspace-index-maintenance-spec.md))
- Go-to-definition / implementation for the newer member types (queries, triggers, indices, storage, etc.)
- Find references and symbol-oriented LSP features
- Formatting support beyond the current refactor rewrites
