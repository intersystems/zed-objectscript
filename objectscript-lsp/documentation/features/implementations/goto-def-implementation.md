# Goto-Definition Implementation

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  LSP Handler (lsp.rs:goto_definition)                       │
│  - Resolves cursor position to tree-sitter node             │
│  - Classifies node via get_outer_type_from_identifier()     │
│  - Dispatches to resolution function by MemberType          │
└────────────────────────────┬────────────────────────────────┘
                             │
         ┌───────────────────┼───────────────────────┐
         │                   │                       │
         ▼                   ▼                       ▼
┌─────────────────┐ ┌────────────────┐ ┌─────────────────────┐
│ get_class_      │ │ get_method_    │ │ get_variable_       │
│ definition()    │ │ definition()   │ │ definition()        │
│                 │ │                │ │                     │
│ get_class_      │ │ get_oref_      │ │ find_classes_       │
│ superclasses()  │ │ definitions()  │ │ from_oref()         │
│                 │ │                │ │                     │
│ get_method_     │ │                │ │                     │
│ superclass()    │ │                │ │                     │
└─────────────────┘ └────────────────┘ └─────────────────────┘
         │                   │                       │
         └───────────────────┼───────────────────────┘
                             ▼
              ┌──────────────────────────────┐
              │  Data Structures             │
              │  - ScopeTree (per document)  │
              │  - GlobalSemanticModel       │
              │  - DependencyGraph           │
              │  - OverrideIndex             │
              └──────────────────────────────┘
```

## Node Classification

When the cursor is on an identifier, the handler calls `get_outer_type_from_identifier` (`common.rs:907`) to determine the `MemberType` of the enclosing construct. This drives which resolution path is used.

| Tree-sitter node kind | MemberType produced | Notes |
|---|---|---|
| `class_name` (in `class_definition`) | `ClassDef` | The class's own name |
| `class_name` (elsewhere) | `Class` | A class reference |
| `method_name` → `oref_method` → `relative_dot_method` | `RelativeMethodCall` | `..MethodName()` |
| `method_name` → `oref_method` (other parent) | `OrefMethod` | `obj.Method()` |
| `method_name` → `routine_tag_call`/`print_argument`/`goto_argument`/`extrinsic_function`/`line_ref` | `RoutineMethodCall` | `Do Label`, `$$Label`, etc. |
| `method_name` → `class_method_call`/`system_defined_function` | `ClassMethodCall` | `##class(Cls).Method()` |
| `method_name` → `method_definition` | `MethodDef` | At the definition site itself |
| `lvn` → `oref_chain_expr`/`class_ref` | `OrefMethod` | The variable portion of `x.Method()` |
| `lvn` (other parent) | `LocalVariable` | |
| `gvn` | `GlobalVariable` | |

Additionally, bare `routine_name` nodes and `numeric_literal` nodes (for line offsets) are handled without going through `MemberType`.

## Resolution Functions

### `get_class_superclasses` (`workspace.rs:1541`)

- Looks up the class in `GlobalSemanticModel` using its `ClassId`
- Iterates over `class.inherited_classes`
- Returns the location(s) of each superclass definition

### `get_method_superclass` (`workspace.rs:1503`)

- Finds the `MethodRef` for the method in the current class
- Looks up `override_index.method_overrides` to find the superclass `MethodRef` it overrides
- Returns the location from the superclass method symbol

### `get_class_definition` (`workspace.rs:1439`)

- Looks up the class name in `self.classes` → `ClassId`
- Retrieves the class symbol from `GlobalSemanticModel`
- Returns the `(url, range)` of the class definition

### `get_method_definition` (`workspace.rs:1134`)

- Looks up `method_defs[class_name][method_name]` → `MethodRef`
- Checks `GlobalSemanticModel` for a public method symbol first
- Falls back to `scope_tree.get_private_method_symbol` for private methods
- If an offset is provided, adjusts the target range by adding the offset to the start row
- Returns the definition location

### `get_oref_definitions` (`workspace.rs:1382`)

- Calls `find_classes_from_oref` to determine what class the oref variable is an instance of
- If `resolve_method = true`: calls `get_method_definition` for each resolved class
- If `resolve_method = false`: returns the variable definition locations directly

### `get_variable_definition` (`workspace.rs:1190`)

Three-tier resolution (see algorithm below).

### `find_classes_from_oref` (`workspace.rs:1627`)

Resolves what class an oref variable is an instance of (see algorithm below).

## Key Data Structures

### ScopeTree (`scope_tree.rs`)

Per-document tree of lexical scopes. Each `Scope` node contains:
- `private_variable_defs`: variable name → `Vec<VariableRef>` (private definitions in this scope)
- `public_var_defs`: variable name → `Vec<VariableRef>` (public definitions)
- `variable_symbols`: `Vec<VariableSymbol>` (location + dependency metadata)
- `children`: child scope IDs
- `method`: optional method name this scope belongs to

The scope tree is used for local variable lookup. `find_current_scope(point)` finds the innermost scope containing a position, and `get_scope_children` collects all nested scopes for recursive search.

### DependencyGraph (`dependency_tracker.rs`)

A directed graph (`petgraph::DiGraph<MethodRef, Range>`) where:
- Nodes = methods (`MethodRef`)
- Edges = caller → callee, weighted by the call-site `Range`

`all_ancestors(target)` performs BFS over incoming edges to find all transitive callers, returning `(MethodRef, call_site_Range, depth)` triples sorted by depth. This enables cross-method variable resolution for public variables.

### OverrideIndex (`override_index.rs`)

Tracks method overriding relationships:
- `method_overrides`: subclass `MethodRef` → superclass `MethodRef` it overrides
- `method_overridden_by`: superclass `MethodRef` → all subclass `MethodRef`s that override it
- `effective_methods`: per-class map of method name → resolved `MethodRef`

Used by `get_method_superclass` for navigating up the inheritance chain.

#### Multiple Inheritance Precedence

`build_override_index_for_classes` builds each class's effective member tables from its parents before overlaying the class's own members. Parent order is significant:
- Default inheritance processes `class.inherited_classes` from left to right.
- `[Inheritance = right]` processes the same list in reverse order.
- Parent members are inserted with first-wins semantics, so the first parent that exposes a method/property/parameter name owns that effective entry.

This means inherited parent members are treated the same as members declared directly on a parent. In the multiple-inheritance regression fixture, `Demo.ChildDefault Extends (Demo.LeftParent, Demo.RightParent)`, `Demo.LeftParent Extends Demo.Base`, `Demo.Base` defines `Common`, and `Demo.RightParent` also defines `Common`. Because default inheritance is left-to-right, `Demo.ChildDefault.Common` must resolve to `Demo.Base.Common`, not `Demo.RightParent.Common`.

#### Late Indexing and Stale Effective Tables

Workspace indexing and `didOpen` events can arrive in an order where a child is indexed before one of its parents. The failing scenario was:
- `Demo.Base` and `Demo.RightParent` were known.
- `Demo.ChildDefault` was indexed while `Demo.LeftParent` was still unresolved, so `Common` could temporarily resolve through `Demo.RightParent`.
- `Demo.LeftParent` was indexed later and inherited `Common` from `Demo.Base`.
- The child effective table was not always rebuilt after that parent became resolvable, so goto-definition could keep returning `Demo.RightParent.Common`.

The fix keeps the override index and the goto-definition lookup maps synchronized when inheritance resolution changes:
- `new_class_inheritance` adds classes collected while resolving unresolved parent references to the full override rebuild set before rebuilding.
- `update_class_inheritance` does the same for inheritance edits by rebuilding the current class, direct dependents, and any additional classes gathered while reconnecting inheritance edges.
- `rebuild_override_index_for_classes_and_apply` wraps `build_override_index_for_classes` and merges returned inherited methods, properties, and parameters into `method_defs`, `property_defs`, and `parameter_defs`.

The final step matters because relative method goto-definition consults `method_defs[class_name][method_name]` before falling back to `override_index.effective_methods`. If the override index is correct but `method_defs` is stale or missing an inherited method, goto-definition can still navigate to the wrong place or fail to navigate.

The regression tests cover both sides of the rule:
- `test_goto_def_multiple_inheritance_default_left` verifies `Demo.ChildDefault.Common` resolves through `Demo.LeftParent` to `Demo.Base`.
- `test_goto_def_multiple_inheritance_late_left_parent_prefers_base` indexes `Base`, `RightParent`, `ChildDefault`, then `LeftParent` to reproduce the stale-state ordering and verifies both `override_index.effective_methods` and `method_defs` point at `Demo.Base.Common`.
- `test_goto_def_multiple_inheritance_right_direction` verifies `[Inheritance = right]` still resolves `Common` through `Demo.RightParent`.

Relevant code paths:
- `ProjectData::new_class_inheritance` and `ProjectData::update_class_inheritance` decide which classes must be rebuilt after inheritance resolution changes.
- `ProjectData::build_override_index_for_classes` computes effective inherited members in parent-precedence order.
- `ProjectData::rebuild_override_index_for_classes_and_apply` keeps the override index and goto-definition lookup maps aligned.
- The regression tests live in `src/test.rs` near the multiple-inheritance goto-definition tests.

### GlobalSemanticModel

Stores workspace-wide symbols:
- Class symbols (`ClassGlobalSymbol`) — name, url, location
- Method symbols (`MethodSymbol`) — for public methods
- Variable symbols (`VariableGlobalSymbol`) — for public variables

### ProjectState Fields

- `method_defs: HashMap<String, HashMap<String, MethodRef>>` — class name → method name → MethodRef
- `pub_var_defs: HashMap<String, HashMap<MethodRef, HashMap<ScopeId, Vec<VariableRef>>>>` — variable name → method → scope → refs
- `classes: HashMap<String, ClassId>` — class name → ClassId

## Variable Resolution Algorithm

`get_variable_definition` (`workspace.rs:1190`):

```
Input: (document_url, cursor_point, variable_name)

1. Find the current method name from scope tree
2. Find the current scope ID from scope tree
3. Look up the MethodRef for this method

TIER 1 — Private variable in scope:
  scope_tree.get_variable_definition(name, scope_id) → Vec<(ScopeId, Vec<Range>)>
  For each (child_scope_id, ranges) where child is current scope or a child of it:
    For each range that ends BEFORE cursor:
      If first definition seen in this scope → record it
      If later definition in same scope → replace (closest-before wins)
  If any found → return

TIER 2 — Public variable in current method's scope:
  scope_tree.pub_variable_in_scope(name, scope_id) → Vec<(ScopeId, Vec<VariableRef>)>
  For each VariableRef with pub_id:
    Resolve via global_semantic_model.get_variable_symbol(method_ref, id, scope_id)
    Same closest-before-cursor logic
  If any found → return

TIER 3 — Public variable from ancestor callers:
  Check is_variable_public(method_ref, variable_name)
  If public:
    Get node_index from dependency_graph
    Get pub_var_defs[variable_name] → all methods that define this variable
    BFS all_ancestors(node_index) → (ancestor_ref, call_range, depth)
    For each ancestor at current depth:
      For each definition in ancestor that ends BEFORE the call site:
        Same closest-before logic
    Stop at first depth that yields results (shallow callers win)
```

## Oref Resolution Algorithm

`find_classes_from_oref` (`workspace.rs:1627`):

```
Input: (oref_variable_name, method_name_called, current_class, call_site_range)

1. Find current document, method, and scope
2. Determine if variable is public via is_variable_public()
3. Look up all oref references in scope via get_oref_references(name, scope_id)

For each variable ref found:
  If PRIVATE:
    Get variable from local_semantic_model
    Check variable.is_oref == true
    Extract variable.cls (the class it was instantiated as)
    Look up method_defs[oref_class][method_called] → MethodRef
    Apply closest-before-cursor logic
  If PUBLIC:
    Get variable from global_semantic_model
    Same is_oref + cls check
    Same method lookup

If public and nothing found locally:
  Traverse DependencyGraph ancestors (same BFS pattern as variable resolution)
  For each ancestor that defines this variable:
    Check is_oref + extract class name
    Resolve method in that class
    Stop at shallowest depth with results

Returns: (Vec<(MethodRef, Range)>, Vec<(Url, Range)>)
  - First vec: the resolved method refs + their call ranges
  - Second vec: the variable definition locations
```

## `is_variable_public` (`workspace.rs:1406`)

Determines if a variable is public (visible across method boundaries):
- `true` if the method has `ProcedureBlock = 0`
- `true` if the class has `ProcedureBlock = 0` and the method doesn't override it
- `true` if the variable is in `method.public_variables_declared`
- `false` otherwise

## Dispatch Logic in LSP Handler

The handler (`lsp.rs:649-1174`) follows this flow:

1. Get document content, tree, class_id, class_name from project state
2. Convert LSP position → tree-sitter Point
3. Find smallest named node at that point
4. If node is `identifier`/`objectscript_identifier`/`objectscript_identifier_special`:
   - Get parent node → classify via `get_outer_type_from_identifier`
   - Match on `MemberType` → dispatch to appropriate resolution function
5. Else if node is `routine_name` → `get_class_definition`
6. Else if node is `gvn` → find identifier child → `get_variable_definition`
7. Else if node is `lvn` (bare, no children match above) → `get_variable_definition`
8. Else if node is `numeric_literal` in a tag call → compute line offset
9. Convert results to LSP Locations:
   - 0 results → `None`
   - 1 result → `GotoDefinitionResponse::Scalar`
   - 2+ results → `GotoDefinitionResponse::Array`

## OrefMethod Sub-dispatch

The `MemberType::OrefMethod` case has additional branching based on the node kind:

- **`method_name`**: cursor is on the method being called
  - Parent is `oref_method` → grandparent is `oref_chain_segment` or `do_parameter`
  - Extract the variable from the first child of the oref expression
  - If variable is `lvn`: call `get_oref_definitions(var, method, class, range, true)`
  - If variable is `class_method_call` with `%New`: extract class directly → `get_method_definition`

- **`lvn`**: cursor is on the object variable itself
  - Parent determines context:
    - `class_ref` → parent is `class_method_call` or `oref_chain_expr` → extract method name → `get_oref_definitions(..., false)`
    - `do_parameter` / `job_argument` → extract method from sibling `oref_method` node → `get_oref_definitions(..., false)`
    - `oref_chain_expr` → extract method from second child → `get_oref_definitions(..., false)`
