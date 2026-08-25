# Diagnostics Implementation

## Entry Point

`src/lsp.rs` — `async fn diagnostic`

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  LSP Handler (lsp.rs:diagnostic)                            │
│  - Gets document snapshot (file_type, content, tree)        │
│  - Checks ProjectData.config diagnostic gates               │
│  - Calls push_host_syntax_diagnostics for all file types    │
│  - For XML: additionally calls                              │
│    push_xml_injected_objectscript_diagnostics               │
│  - In strict mode: adds project semantic diagnostics        │
│  - Returns FullDocumentDiagnosticReport                     │
└────────────────────────────┬────────────────────────────────┘
                             │
              ┌──────────────┼──────────────┐
              │                             │
              ▼                             ▼
┌──────────────────────────┐  ┌──────────────────────────────┐
│ push_host_syntax_        │  │ push_xml_injected_           │
│ diagnostics()            │  │ objectscript_diagnostics()   │
│                          │  │                              │
│ - collect_error_nodes()  │  │ - xml_objectscript_          │
│ - diagnostic_message()   │  │   implementation_ranges()    │
│                          │  │ - Parse each range as OS     │
│                          │  │ - push_host_syntax_          │
│                          │  │   diagnostics() per range    │
└──────────────────────────┘  └──────────────────────────────┘
```

## Server Capabilities

Registered in `build_caps` in `src/lsp.rs`:

```rust
diagnostic_provider: Some(DiagnosticServerCapabilities::Options(DiagnosticOptions {
    identifier: None,
    inter_file_dependencies: true,
    workspace_diagnostics: true,
    work_done_progress_options: Default::default(),
}))
```

This registers pull-based diagnostics (client requests via `textDocument/diagnostic`), not push-based.

## Handler Flow (`src/lsp.rs`)

```
1. Get document URI from params
2. Get project from document URL
3. Read the workspace `ProjectData`
4. Return no diagnostics if `data.config.enable_lint` is false
5. Call push_host_syntax_diagnostics(diagnostics, content, tree, file_type)
6. If file_type == XML, call push_xml_injected_objectscript_diagnostics(diagnostics, content, tree)
7. If `data.config.enable_strict_mode` is true, add project semantic diagnostics
8. Return FullDocumentDiagnosticReport with the selected diagnostics
```

## Configuration Flow

Startup config is parsed from `initialize.initializationOptions` with `Config::from_lsp_value` and stored in each workspace's `ProjectData.config`.

Runtime config changes are handled by `did_change_configuration`:

```
1. If the client supports workspace/configuration, request current settings
2. Parse the first response that contains ObjectScript config keys with Config::from_lsp_value_if_present
3. If workspace/configuration is unavailable, parse the notification's settings payload
4. Ignore empty payloads and unrelated LSP settings
5. Apply the parsed config to each workspace ProjectData
6. Refresh workspace diagnostics when the effective config changed
```

This guard matters because many clients send `workspace/didChangeConfiguration` with an empty payload, or return broad LSP settings from `workspace/configuration`. Those payloads must not be treated as `Config::default()`, because that would reset `enable_strict_mode` to `true` and re-enable semantic diagnostics.

## Key Functions

### `push_host_syntax_diagnostics` (`src/lsp.rs`)

Finds all error nodes in the document's tree-sitter parse tree and converts them to LSP diagnostics.

```
1. Call collect_error_nodes(tree.root_node()) → Vec<Node>
2. For each error node:
   a. Convert node range to LSP range via ts_range_to_lsp_range
   b. Get error text from node byte range
   c. Generate message:
      - If XML: "XML syntax error: Unexpected {text}"
      - Else: try diagnostic_message(node, text) for context-aware message
      - Fallback: "Syntax Error: Unexpected {text}"
   d. Push Diagnostic with severity ERROR
```

### `push_xml_injected_objectscript_diagnostics` (`src/lsp.rs`)

Finds ObjectScript code embedded in XML `<Implementation>` blocks and runs syntax checking on each.

```
1. Call xml_objectscript_implementation_ranges(tree.root_node(), content) → Vec<Range>
2. For each range:
   a. Extract text at range, skip if empty/whitespace
   b. Create fresh Parser with ObjectScript language
   c. Set parser included_ranges to [range] (restricts parsing to that region)
   d. Parse content with the constrained parser
   e. Call push_host_syntax_diagnostics on the resulting tree (as FileType::Routine)
```

### `collect_error_nodes` (`common.rs:1082`)

Recursively walks the tree-sitter tree collecting error and missing nodes.

```
1. Start with root node, walk with TreeCursor
2. visit_errors(node, cursor, out):
   a. If node has no error, is not error, and is not missing → return (prune subtree)
   b. If node is_error() or is_missing() → push to output
   c. Recurse into children
```

The early return on `!node.has_error()` prunes healthy subtrees for efficiency.

### `xml_objectscript_implementation_ranges` (`common.rs:1090`)

Uses a tree-sitter query to find ObjectScript code inside XML Implementation elements.

```
1. Create Query from XML_OBJECTSCRIPT_INJECTIONS_QUERY
2. Run query against XML tree root
3. For each match, find capture named "injection.content"
4. Push non-empty ranges to output
5. Sort and deduplicate by (start_byte, end_byte)
```

**Query pattern (`common.rs:14`):**
```scheme
(element
  (STag
    (Name) @_name)
  (content
    (CDSect
      (CData) @injection.content))
  (#eq? @_name "Implementation")
  (#set! injection.language "objectscript"))

(element
  (STag
    (Name) @_name)
  (content
    (CharData) @injection.content)
  (#eq? @_name "Implementation")
  (#set! injection.language "objectscript"))
```

Two patterns handle both `<![CDATA[...]]>` wrapped content and plain text content within `<Implementation>` tags.

### `diagnostic_message` (`src/common.rs:4`)

Generates context-aware error messages by inspecting the previous sibling of the error node. Currently only provides enhanced messages for `command_set` errors:

```
1. Get prev_named_sibling of error node
2. If sibling is a "statement":
   a. Get its first child command
   b. If "command_set":
      - Get last child of command_set
      - Match on child kind:
        - "keyword_set" → "Expected a variable name, got {text}"
        - "set_argument" → inspect further:
          - Last child is set_target with = sibling → "Expected an expression..."
          - Last child is set_target without = → "Expected '=' or another variable name..."
          - Last child is expression → "Unexpected {text} after expression..."
3. Return None if no context-aware message applies (falls back to generic message)
```

## Diagnostic Properties

All diagnostics produced share these properties:

| Property | Value |
|---|---|
| `severity` | `DiagnosticSeverity::ERROR` |
| `code` | `None` |
| `source` | `None` |
| `related_information` | `None` |
| `tags` | `None` |

## XML Mixed-Language Strategy

The XML diagnostic pass uses parser `set_included_ranges` to constrain the ObjectScript parser to only the bytes within an `<Implementation>` block. This means:
- The parser sees the raw content bytes at their original offsets in the file
- Error ranges reported by the ObjectScript parser map directly to the correct positions in the XML document
- No offset translation is needed — the LSP range conversion works on the same content string
