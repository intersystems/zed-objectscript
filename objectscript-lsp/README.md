# ObjectScript LSP

Language Server & Language Server Protocol implementation for InterSystems ObjectScript using `tower-lsp` and `tree-sitter`.

We built this language server to provide editor-independent ObjectScript semantics for VS Code, Zed, Neovim, and other LSP clients without requiring a live InterSystems server connection.

## Current Features

- Workspace indexing for `.cls`, `.inc`, `.rtn`, `.mac`, and `.int`
- Everything is rebuilt incrementally.
- Multi-workspace support through LSP workspace folders, with deepest-parent routing per document
- Go-to-definition for ObjectScript variables, orefs, methods, properties, classes, parameters with ProcedureBlock-aware private/public resolution
- Go-to-implementation for inherited and overridden methods and classes
- Syntax diagnostics for tracked ObjectScript documents
- Mixed-language diagnostics for ObjectScript captured from XML `Implementation` blocks
- Refactor code actions for:
  - Legacy dotted `DO` rewrites
  - Legacy `IF/Else` rewrites
  - Legacy `FOR` rewrites
  - document-scoped and workspace-scoped edits
- Inheritance modeling and override index build
- Dependency modeling and dependencyGraph build (shows all paths to a given method)


## Architecture Summary

- One `ProjectState` is created per workspace folder
- Two-phase semantic build:
  - initial class/routine parse and symbol creation
  - inheritance, variables, and call extraction
- Public symbols live in `GlobalSemanticModel`
- Private symbols are tracked through `LocalSemanticModel` and `ScopeTree`
- XML documents are tracked for diagnostics, but they do not enter the class/routine semantic rebuild pipeline

## Workspace Layout

- `objectscript-lsp`: LSP transport layer in [src/main.rs](src/main.rs), [src/lsp.rs](src/lsp.rs), and [src/server.rs](src/server.rs)
- `crates/objectscript-core`: parsing, semantic model, workspace state, refactors, and dependency tracking
- `objectscript-tests/`: fixture corpus for inheritance, dependencies, navigation, and related regressions

## LSP Surface

- Standard requests:
  - `textDocument/definition`
  - `textDocument/implementation`
  - `textDocument/diagnostic`
- Code actions and execute commands for refactor rewrites

## Configuration

Editor-specific configuration examples for Zed, Neovim, and VS Code are documented in [documentation/configuration.md](documentation/configuration.md).


## Build and Test

```bash
cargo build
cargo test
```

### Go-To Definition

Go-to definition works for classes, class methods, orefs, procedures, subroutines, instance methods, public local variables, private local variables, and global variables. 

For variables, the way the definition(s) is determined depends on the case: 
**CASE 1: Variable is defined the current scope. **
In this case, the variable definition in the given scope is returned.

**CASE 2: Variable is NOT defined the current scope. **
**CASE 2A: Variable is Private**
This means that the variable is undefined. No definition is returned.
**CASE 2B: Variable is Public**
In this case, the `DependencyGraph` is used to determine all possible paths to the current scope. For each node (scope) on the path, we check if the wanted variable is defined in that scope, and if so we track that location. All possible locations are returned.


## Grammar Baseline

- `tree-sitter = 0.26.6`
- `tree-sitter-objectscript = 1.9.20`
- `tree-sitter-objectscript-routine = 1.9.20`
- `tree-sitter-objectscript-playground = 1.9.20`
- `tree-sitter-xml = 0.7.0`

## Roadmap

- Semantic diagnostics (undefined variables, unresolved symbols)
- Broader mixed-language support beyond XML `Implementation` blocks
- More lifecycle and incremental edit coverage
- Expanded semantic support for properties, parameters, queries, triggers, and storage
- Find references and symbol-oriented LSP features
- Formatting support beyond the current refactor rewrites
