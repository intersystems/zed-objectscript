# Diagnostics Feature

## Overview

The diagnostics feature reports syntax errors in ObjectScript and XML files. It uses tree-sitter parse trees to detect error nodes and reports them as LSP diagnostics with severity `ERROR`.

## Supported File Types

| File Type | Diagnostics Provided |
|---|---|
| Class (`.cls`) | ObjectScript syntax errors |
| Routine (`.mac`, `.int`, `.rtn`, `.inc`) | ObjectScript syntax errors |
| XML (`.xml`) | XML syntax errors + ObjectScript errors in `<Implementation>` blocks |

## Diagnostic Types

### ObjectScript Syntax Errors

Reported when tree-sitter produces error or missing nodes in the parse tree. Each error node becomes a diagnostic at that node's source range.

**Context-aware messages** — when the error occurs in a recognizable context, a more helpful message is produced:

| Context | Message |
|---|---|
| After `Set` keyword with no target | "Expected a variable name, got {text}" |
| After `set_target` with no `=` | "Expected '=' or another variable name..." |
| After `=` in `Set` with no expression | "Expected an expression, {text} is not a valid expression." |
| After expression in `Set` | "Unexpected, {text} after an expression. Expected a binary operator or end of SET command" |
| All other cases | "Syntax Error: Unexpected {text}" |

### XML Syntax Errors

For XML files, tree-sitter XML error nodes produce diagnostics with the message format: `"XML syntax error: Unexpected {text}"`.

### Mixed-Language Diagnostics (XML + ObjectScript)

For XML files that contain ObjectScript code inside `<Implementation>` CDATA blocks, the LSP performs a second diagnostic pass:

1. Finds all `<Implementation>` elements in the XML tree
2. Extracts the CDATA or CharData content from each
3. Parses each extracted region as ObjectScript using an independent parser
4. Reports any ObjectScript syntax errors found within those regions

This provides syntax checking for ObjectScript code that is embedded within XML class export files.

### Project Semantic Diagnostics

Project semantic diagnostics are produced from workspace indexes and cross-document semantic state. These include unresolved method-reference diagnostics such as `"Method referenced has either not yet been indexed or does not exist"`.

These diagnostics are included only when `enableStrictMode` is `true`.

## Behavior

- Diagnostics are computed on-demand via the `textDocument/diagnostic` pull model (not push-based `textDocument/publishDiagnostics`)
- Each diagnostic request returns a full report for the requested document
- Workspace-level diagnostics are returned via `workspace/diagnostic` for tracked documents
- `enableLint: false` disables diagnostics
- `enableStrictMode: false` keeps syntax diagnostics enabled but filters out project semantic diagnostics, including unresolved method-reference diagnostics
- Runtime changes sent through `workspace/didChangeConfiguration` update diagnostic behavior without restarting the server
- Empty configuration notifications and unrelated LSP settings are ignored so they do not reset strict mode to its default
