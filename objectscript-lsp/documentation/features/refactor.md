# Refactor Feature (Code Actions -> Refactor)

## Refactoring Types

### 1. Legacy Do Commands (Dotted Do → Subroutines)

Converts legacy dotted `Do` blocks into named private subroutines with explicit `Do subroutineName` calls.

**Before:**
```objectscript
Main
    do
    . set x = 1
    . write x
    write "done"
    q
```

**After:**
```objectscript
Main
    do MainSubroutine1
    write "done"
    q
    
MainSubroutine1 Private
    set x = 1
    write x
    quit
```

**Details:**
- The generated subroutine name is derived from the enclosing label/procedure name + `Subroutine` + depth number (e.g., `MainSubroutine1`)
- If the name conflicts with an existing routine member, the depth number is incremented
- If the do block contains `$TEST`, `JOB`, `LOCK`, `OPEN`, or `READ` (commands that modify `$TEST`), the generated subroutine saves and restores `$TEST` via a `temp` variable
- A trailing `quit` is appended unless the body already ends with `quit` or `return`
- Comments are preserved in the generated call and subroutine
- After extraction, subroutine spacing/indentation is normalized
- Only available for routine files (`.mac`, `.int`, `.rtn`, `.inc`)

---

### 2. Legacy If/Else Commands (Single-line → Block Form)

Converts legacy single-line `If`/`Else` conditionals into modern block-form `if`/`else` with curly braces.

**Before:**
```objectscript
    If x=1 Write "yes"
    Else  Write "no"
```

**After:**
```objectscript
    if x=1 {
       Write "yes"
    }
    else {
       Write "no"
    }
```

**Rules:**
- If the `If` has no expression and no statements (unreachable), the entire `If`/`Else` pair is removed
- Argumentless `If` (no expression) with statements → becomes `if $TEST`
- If the `If` has an expression but no statements, and there's an `Else` with statements → transforms into `if '(expression)` (negated condition) with the else body
- Adjacent `If`/`Else` pairs are refactored together into `if/else` blocks
- Standalone old `Else` without a paired `If` → becomes `if $TEST = 0`
- Comments between keywords and statements are preserved
- Available for both class files and routine files

---

### 3. Legacy For Commands (Single-line → Block Form)

Converts legacy single-line `For` loops into modern block-form with curly braces.

**Before:**
```objectscript
    For i=1:1:10 Write i
```

**After:**
```objectscript
    for i=1:1:10 {
       Write i
    }
```

**Rules:**
- If the `For` has no parameters and no statements (unreachable), it is removed
- The for parameter (loop variable/range) is preserved as-is
- Comments are preserved
- Available for both class files and routine files

---

## Scope Options

Each refactoring type can be applied at two scopes:

| Scope | Behavior |
|---|---|
| **Document** | Refactors only the current file |
| **Workspace** | Refactors all applicable files in the workspace |

## Available Refactoring Levels per File Type

| File Type | Available Levels |
|---|---|
| Routine (`.mac`, `.int`, `.rtn`, `.inc`) | Do Commands, Conditionals, For Commands, All |
| Class (`.cls`) | Conditionals, For Commands, All |
| XML (`.xml`) | None |

The "All" level applies all applicable transformations in sequence: Do Commands (if routine) → Conditionals → For Commands.

## Preconditions

- **Document must parse without errors** — document-level refactors are only offered when the tree-sitter parse has no error nodes
- **Workspace refactors are manually triggered only** — they don't appear in automatic code action suggestions (only on explicit user request)

## Behavior

- If the refactoring produces no changes (code is already in modern form), the code action is a no-op
- The refactoring replaces the entire document content with the updated version via a `workspace/applyEdit` request
- If the editor rejects the edit, a warning message is logged
