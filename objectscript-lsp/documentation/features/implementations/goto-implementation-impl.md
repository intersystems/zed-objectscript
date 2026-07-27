# Goto-Implementation Implementation

## Entry Point

`lsp.rs:1176` — `async fn goto_implementation`

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  LSP Handler (lsp.rs:goto_implementation)                   │
│  - Resolves cursor position to tree-sitter node             │
│  - Classifies node via get_outer_type_from_identifier()     │
│  - Dispatches by MemberType (Class, ClassMethodCall,        │
│    MethodDef only)                                          │
└────────────────────────────┬────────────────────────────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
              ▼              ▼              ▼
   ┌──────────────┐  ┌─────────────┐  ┌─────────────┐
   │ get_class_   │  │ get_method_ │  │ get_method_ │
   │ implementa-  │  │ overrides() │  │ overrides() │
   │ tions()      │  │ (from call) │  │ (from def)  │
   └──────────────┘  └─────────────┘  └─────────────┘
              │              │              │
              └──────────────┼──────────────┘
                             ▼
              ┌──────────────────────────────┐
              │  Data Structures             │
              │  - OverrideIndex             │
              │  - Dependents (class index)  │
              │  - GlobalSemanticModel       │
              └──────────────────────────────┘
```

## Node Classification

Only three `MemberType` variants are handled; all others return `Ok(None)`:

| Tree-sitter node kind | MemberType | Handling |
|---|---|---|
| `class_name` (any) | `Class` | Further branched by parent node kind |
| `method_name` → `class_method_call` | `ClassMethodCall` | Method override lookup |
| `method_name` → `method_definition` | `MethodDef` | Method override lookup |

## Resolution Paths

### 1. Class (`MemberType::Class`)

Two sub-cases based on the parent node of `class_name`:

#### 1a. Class name in `class_definition` (own definition)

Uses the current document's `class_id` directly.

**Resolution:** `get_class_implementations` (`workspace.rs:1457`)
- Looks up `dependent_class_index.dependent_classes[class_id]` → `Vec<ClassId>`
- For each dependent class ID, retrieves the class symbol from `GlobalSemanticModel`
- Returns all `(url, range)` pairs for the subclass definitions

#### 1b. Class name as a reference (elsewhere)

Extracts the full class name string from the `class_name` node, then looks up its `ClassId` from `self.classes`.

**Resolution:** Same `get_class_implementations` with the looked-up `ClassId`.

---

### 2. Class Method Call (`MemberType::ClassMethodCall`)

When the cursor is on the method name in `##class(ClassName).MethodName()`.

**Resolution:**
- Extracts the class name from the `class_ref` child node
- Looks up `method_defs[class_name][method_name]` → `MethodRef`
- Calls `get_method_overrides` (`workspace.rs:1576`)

---

### 3. Method Definition (`MemberType::MethodDef`)

When the cursor is on a method name at its definition site.

**Resolution:**
- Gets the class from `GlobalSemanticModel` using the current `class_id`
- Looks up `class.methods[method_name]` → `MethodRef`
- Calls `get_method_overrides`

---

## Resolution Functions

### `get_class_implementations` (`workspace.rs:1457`)

```
Input: class_id

1. Look up dependent_class_index.dependent_classes[class_id] → Vec<ClassId>
2. For each dependent ClassId:
   a. Get class struct from GlobalSemanticModel → extract class name
   b. Look up class name in self.classes → ClassId (symbol id)
   c. Get class symbol from GlobalSemanticModel → (url, range)
3. Return all locations
```

### `get_method_overrides` (`workspace.rs:1576`)

```
Input: method_ref (the superclass method)

1. Look up override_index.overridden_by[method_ref] → Vec<MethodRef>
2. For each overriding MethodRef:
   a. Try GlobalSemanticModel.get_method_symbol → (url, range) [public methods]
   b. Fallback: get document by class url → scope_tree.get_private_method_symbol → (url, range) [private methods]
3. Return all locations
```

## Key Data Structures

### Dependents (`dependency_tracker.rs`)

```rust
pub struct Dependents {
    pub dependent_classes: HashMap<ClassId, Vec<ClassId>>,
}
```

Maps each class to all classes that directly inherit from it. Built during workspace indexing when inheritance relationships are resolved.

### OverrideIndex (`override_index.rs`)

The `overridden_by` field is the key structure for this feature:

```rust
pub overridden_by: HashMap<MethodRef, Vec<MethodRef>>
```

Maps a superclass method → all subclass methods that override it. This is the inverse of the `overrides` map used by goto-definition.

## Dispatch Logic

The handler (`lsp.rs:1176-1361`) follows this flow:

1. Get document content, tree, class_id from project state
2. Convert LSP position → tree-sitter Point
3. Find smallest named node at that point
4. If node is `identifier` or `objectscript_identifier`:
   - Get parent node → classify via `get_outer_type_from_identifier`
   - Match on `MemberType`:
     - `Class` → branch on parent kind → `get_class_implementations`
     - `ClassMethodCall` → extract class + method → `get_method_overrides`
     - `MethodDef` → get method from class → `get_method_overrides`
     - All others → return `None`
5. Convert results to LSP Locations:
   - 0 results → warning message + `None`
   - 1 result → `GotoImplementationResponse::Scalar`
   - 2+ results → `GotoImplementationResponse::Array`
