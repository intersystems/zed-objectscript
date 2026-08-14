# Refactor Feature Implementation

## Entry Points

- **Code Action Provider:** `lsp.rs:410` — `async fn code_action`
- **Command Executor:** `lsp.rs:507` — `async fn execute_command`
- **Core Logic:** `refactor.rs` (1687 lines)

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Code Action Provider (lsp.rs:code_action)                  │
│  - Checks if refactor.rewrite kind is requested             │
│  - Builds menu of available refactor commands per file type │
│  - Returns CodeAction list with command references          │
└────────────────────────────┬────────────────────────────────┘
                             │ user selects action
                             ▼
┌─────────────────────────────────────────────────────────────┐
│  Execute Command (lsp.rs:execute_command)                   │
│  - Parses URI + RefactorLevel from command arguments        │
│  - Dispatches to document or workspace refactor             │
│  - Sends workspace/applyEdit to editor                      │
└────────────────────────────┬────────────────────────────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
              ▼              ▼              ▼
   ┌──────────────┐  ┌─────────────┐  ┌─────────────┐
   │ refactor_    │  │ refactor_   │  │ refactor_   │
   │ legacy_do_  │  │ conditionals │   │ for_        │
   │ statements()│  │ ()          │   │ statements()│
   └──────────────┘  └─────────────┘  └─────────────┘
```

## LSP Integration

### Commands

| Command ID | Scope |
|---|---|
| `objectscript.refactorDocument` | Single file |
| `objectscript.refactorWorkspace` | All workspace files |
| `objectscript.refactorWorkspaceDottedDo` | Legacy alias (Do only, workspace) |

### Command Arguments

Each command receives two arguments:
1. Document URI (string)
2. Refactor level: `"all"`, `"do"`, `"conditionals"`, or `"for"`

### Code Action Generation (`lsp.rs:410`)

```
1. Check refactor_kind_requested — only proceed if refactor.rewrite is in the requested kinds
2. Get document file type and parse tree
3. If document has no parse errors:
   - Add document-scoped refactor actions for each applicable level
   - "Refactor All Code in this document" is marked as is_preferred
4. If not XML and not an automatic trigger:
   - Add workspace-scoped refactor actions for all 4 levels
5. Return the action list
```

### Execute Command (`lsp.rs:507`)

```
1. Parse URI from arguments[0]
2. Parse RefactorLevel from arguments[1] (or command name for legacy)
3. Get project from document URL
4. If document command:
   - Call build_document_refactor_edit → full-document TextEdit
   - Send workspace/applyEdit
5. If workspace command:
   - Call collect_workspace_refactor_changes → HashMap<Url, Vec<TextEdit>>
   - Send workspace/applyEdit with all changes
```

### Helper Functions (`lsp.rs`)

| Function | Location | Purpose |
|---|---|---|
| `refactor_kind_requested` | `lsp.rs:74` | Checks if `refactor.rewrite` is in the requested CodeActionKinds |
| `refactor_title` | `lsp.rs:100` | Generates display title (e.g., "Refactor Legacy Do Commands in workspace") |
| `selectable_document_refactor_levels` | `lsp.rs:110` | Returns available levels per file type |
| `build_refactor_command` | `lsp.rs:195` | Builds LSP Command struct with title + arguments |
| `build_document_refactor_edit` | `lsp.rs:211` | Creates WorkspaceEdit for single document |
| `collect_workspace_refactor_changes` | `lsp.rs:229` | Creates WorkspaceEdit for all workspace documents |
| `command_refactor_level_argument` | `lsp.rs:258` | Parses level from command arguments |

## Core Refactoring Logic (`refactor.rs`)

### Shared Infrastructure

#### `update_tree_and_content` (`refactor.rs:10`)

Performs an in-place replacement on both the content string and the tree-sitter tree (via `InputEdit`). All refactoring operations use this to maintain tree consistency during multi-step transformations.

#### `build_old_statement_struct` (`refactor.rs:579`)

Parses a legacy command node into an `OldStatement` struct:

```rust
pub struct OldStatement {
    pub last_expression_end_byte: Option<usize>,    // end of condition/for-param
    pub last_expression_end_point: Option<Point>,
    pub statement_ranges: Vec<Range<usize>>,         // body statement byte ranges
    pub keyword_old_range: tree_sitter::Range,       // the legacy keyword node
    pub command_range: tree_sitter::Range,           // entire command node
    pub comment_range: Option<tree_sitter::Range>,   // comment before statements
    pub comment_after_last_statement_range: Option<tree_sitter::Range>,
    pub statements_after: Vec<Range<usize>>,         // do_statement_after nodes
}
```

This struct captures the structural components of legacy statements needed for transformation.

#### `build_replacement_string_block` (`refactor.rs:787`)

Formats statements into a block with curly braces:
```
 {
   statement1
   statement2
}
```

Uses 3-space indentation inside blocks relative to the base indent.

#### `remove_unreachable_statements` (`refactor.rs:43`)

Uses tree-sitter queries to find legacy statements with no body (no expression + no statement), then removes them entirely. Processes removals in reverse byte order to maintain valid positions.

---

### 1. Legacy Do Refactoring

**Public entry:** `refactor_legacy_do_statements` (`refactor.rs:1549`)

**Algorithm:**

```
1. Parse content into tree-sitter tree
2. Collect existing routine member names (labels, tags, procedures)
3. Loop:
   a. Query for (command_do (keyword_do_old)) nodes
   b. Find the first one that actually has a dotted body
      (is_old_do_with_dotted_body checks for lines with expected dot depth)
   c. Find the enclosing subroutine/procedure name
   d. Generate a unique subroutine name (e.g., MainSubroutine1)
   e. Build the new subroutine body:
      - Strip dot prefixes from each line
      - Track brace depth for indentation
      - If $TEST-modifying commands present: wrap with temp=$TEST / $TEST=temp
      - Append quit if body doesn't end with quit/return
   f. Build the replacement call: "do SubroutineName" + any do_statement_after nodes
   g. Insert generated subroutine after the enclosing routine member
   h. Replace the old do block with the new call
   i. Re-parse tree and continue loop
4. After all do blocks are extracted:
   - Normalize spacing for all subroutines (refactor_spacing_for_subroutines)
```

**Key functions:**

| Function | Purpose |
|---|---|
| `direct_dotted_body_depth` | Determines the dot depth of lines following a `do` command |
| `dotted_body_line_ranges` | Collects byte ranges of all dotted body lines at the expected depth |
| `strip_dotted_prefix` | Removes leading dots from a line, returning clean content |
| `build_generated_dotted_do` | Builds the full subroutine text (name + body) |
| `build_new_do_call` | Builds the replacement `do SubroutineName` call with any after-statements |
| `generate_subroutine_name` | Creates a unique name by appending/incrementing `Subroutine{N}` |
| `find_do_statement_subroutine` | Walks up/backward to find the enclosing tag/procedure name and range |
| `changes_test_variable` | Checks if the do block contains `$TEST`/`JOB`/`LOCK`/`OPEN`/`READ` |
| `refactor_spacing_for_subroutines` | Normalizes indentation after extraction (4-space base indent) |
| `has_routine_member_between` | Determines where to insert the generated subroutine |

**Processing order:** The algorithm processes dotted do statements from top to bottom (sorted by `start_byte`), but only processes one per loop iteration (re-parsing after each change).

---

### 2. Conditional Refactoring

**Public entry:** `refactor_conditionals` (`refactor.rs:850`)

**Algorithm:**

```
1. Parse content with appropriate language (UDL for .cls, routine grammar for .mac/.int etc.)
2. Remove unreachable conditionals:
   - Old if with no expression AND no statement → remove
   - Old else with no statement → remove
3. Refactor if-else pairs (loop until no more changes):
   - Query: source_file > statement(command_if(keyword_old_if)) followed by statement(command_else)
   - For each pair: build block-form if/else
4. Refactor standalone old if (loop until no more changes):
   - Query: command_if(keyword_old_if)
   - Convert to block form
5. Refactor standalone old else (loop until no more changes):
   - Query: command_else(keyword_oldelse)
   - Convert to "if $TEST = 0 { ... }" block form
```

**Key functions:**

| Function | Purpose |
|---|---|
| `remove_unreachable_conditionals` | First pass: removes dead if/else statements |
| `refactor_old_conditional_command` | Orchestrates all three conditional sub-passes |
| `refactor_if_else_statements` | Handles paired if+else → if/else block |
| `refactor_old_if_statements` | Handles standalone old if → if block |
| `refactor_old_else_statements` | Handles standalone old else → if $TEST = 0 block |

**If-else transformation rules:**
- If the `if` has no expression: insert `$TEST` as condition
- If the `if` has expression but no body: negate expression, use else body as if body
- If both have bodies: create full `if (expr) { ... } else { ... }` block

---

### 3. For Statement Refactoring

**Public entry:** `refactor_for_statements` (`refactor.rs:822`)

**Algorithm:**

```
1. Parse content with appropriate language
2. Remove unreachable for statements:
   - Old for with no parameter AND no statement → remove
3. Refactor old for (loop until no more changes):
   - Query: command_for(keyword_old_for)
   - Convert to block form with curly braces
```

**Key functions:**

| Function | Purpose |
|---|---|
| `remove_unreachable_for_statements` | First pass: removes dead for statements |
| `refactor_legacy_for_statements` | Loop converting old for → block form |
| `refactor_old_for_statements` | Single transformation of one for statement |

---

## Workspace-Level Refactoring (`workspace.rs`)

### `refactor_document` (`workspace.rs:171`)

```
1. Get document file type and content
2. Skip XML files (no refactoring supported)
3. Dispatch by RefactorLevel:
   - DoCommands → refactor_legacy_do_statements (routine only)
   - Conditionals → refactor_conditionals
   - ForCommands → refactor_for_statements
   - All → chain all three in sequence
4. If output == input → return None (no changes)
5. Return updated content
```

### `refactor` (`workspace.rs:219`)

```
1. Collect applicable document URLs:
   - DoCommands: only routine files
   - Others: all documents
2. For each URL: call refactor_document
3. Return Vec<(updated_content, url)> for all changed files
```

## Data Flow

```
User selects "Refactor All Code in workspace"
  → execute_command receives (uri, "all")
  → collect_workspace_refactor_changes calls project.refactor(RefactorLevel::All)
  → for each document:
    → refactor_legacy_do_statements (if routine)
    → refactor_conditionals (on result)
    → refactor_for_statements (on result)
  → returns HashMap<Url, Vec<TextEdit>>
  → workspace/applyEdit sent to editor
  → editor applies all changes atomically
```

## Tree-Sitter Queries Used

| Refactoring | Query Pattern | Purpose |
|---|---|---|
| Do commands | `(command_do (keyword_do_old)) @command` | Find legacy dotted do blocks |
| If (unreachable) | `(command_if (keyword_old_if) (expression)? @condition (statement)? @statement) @command_if` | Find removable if statements |
| Else (unreachable) | `(command_else (keyword_oldelse) (statement)? @statement) @command` | Find removable else statements |
| If-else pairs | `(source_file (statement (command_if (keyword_old_if)) @command_if) . (statement (command_else) @command_else))` | Find adjacent if/else to pair |
| Standalone if | `(command_if (keyword_old_if)) @command` | Find remaining old if |
| Standalone else | `(command_else (keyword_oldelse)) @command_else` | Find remaining old else |
| For (unreachable) | `(command_for (keyword_for) (for_parameter)? @param (statement)? @statement) @command` | Find removable for (new keyword) |
| For (old, unreachable) | `(command_for (keyword_old_for) (for_parameter)? @param (statement)? @statement) @command` | Find removable for (old keyword) |
| For (old) | `(command_for (keyword_old_for)) @command` | Find remaining old for to convert |

## Processing Strategy

All refactoring passes use the same loop-until-stable pattern:
1. Query for the first matching node
2. Transform it (in-place string replacement + tree edit)
3. Re-parse the tree
4. Repeat until no matches found

This one-at-a-time approach ensures each transformation works on a valid tree, since byte offsets shift after each replacement.
