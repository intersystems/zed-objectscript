# ObjectScript LSP Test Suite

## 1. Goto Definition

### 1.1 Classes — Method Definitions (Superclass Resolution)

| # | Scenario | Expected |
|---|----------|----------|
| 1.1.1 | Click method name in a `class_definition` that overrides a superclass method | Jump to superclass method definition |
| 1.1.2 | Click method name that is NOT defined in any superclass | Return None |
| 1.1.3 | Multiple inheritance (default left): click method defined in both parents | Jump to left parent's definition |
| 1.1.4 | Multiple inheritance `[Inheritance = right]`: click method defined in both parents | Jump to right parent's definition |
| 1.1.5 | Method with `objectscript_identifier_special` (e.g., `%New`) | Resolves correctly |
| 1.1.6 | Subclass opened before superclass is indexed | Still resolves after superclass is loaded |

### 1.2 Classes — Class References

| # | Scenario | Expected |
|---|----------|----------|
| 1.2.1 | Click class name in `Extends` clause | Jump to that class's `class_definition` node |
| 1.2.2 | Click class name in `##class(ClassName)` | Jump to class definition |
| 1.2.3 | Click class name that doesn't exist in workspace | Return None |
| 1.2.4 | Routine name in `do label^routine` | Jump to routine definition |

### 1.3 Classes — Class Method Calls

| # | Scenario | Expected |
|---|----------|----------|
| 1.3.1 | `##class(Foo).Bar()` — click on `Foo` | Jump to class Foo definition |
| 1.3.2 | `##class(Foo).Bar()` — click on `Bar` | Jump to method `Bar` in class `Foo` |
| 1.3.3 | Method doesn't exist in target class | Return None |

### 1.4 Classes — Oref (Object Reference) Methods

| # | Scenario | Expected |
|---|----------|----------|
| 1.4.1 | `set x = ##class(Foo).%New()` then `d x.Method()` — click `x` | Jump to definition of `x` |
| 1.4.2 | Same — click `Method` | Jump to `Method` definition in class `Foo` |
| 1.4.3 | Oref in `do_parameter` context: `do obj.Run()` | Same resolution as above |
| 1.4.4 | Oref in `job_argument` context: `job obj.Execute()` | Same resolution as above |
| 1.4.5 | Multi-segment oref chains (>2 children) | Currently unsupported, returns None |

### 1.5 Classes — Relative Method Calls

| # | Scenario | Expected |
|---|----------|----------|
| 1.5.1 | `d ..MethodName()` — click on `MethodName` | Jump to method def in current class |
| 1.5.2 | Relative call to method that doesn't exist in current class | Return None |

### 1.6 Variables

| # | Scenario | Expected |
|---|----------|----------|
| 1.6.1 | Private variable in `[ProcedureBlock=1]` method — click on usage | Jump to `set` definition in same method |
| 1.6.2 | Public variable (class is `[Not ProcedureBlock]`) — click on usage in method where defined | Jump to local definition |
| 1.6.3 | Public variable — click on usage in method where NOT defined | Return all reachable definitions from dependency graph |
| 1.6.4 | Public variable — unreachable method's definition is NOT returned | Verify via dependency graph path analysis |
| 1.6.5 | Variable in method header (`Pub` argument) | Look in method first, then workspace |
| 1.6.6 | Global variable (`^gvn`) — click on identifier inside gvn node | Jump to definition if in scope |
| 1.6.7 | Variable defined in multiple reachable methods | Return all reachable definitions |

### 1.7 Routines — Goto Definition

| # | Scenario | Expected |
|---|----------|----------|
| 1.7.1 | `do label` (tag call within same routine) | Jump to label definition |
| 1.7.2 | `do label^routine` (cross-routine tag call) | Jump to label in other routine |
| 1.7.3 | `goto label` | Jump to label definition |
| 1.7.4 | Numeric offset: `do label+N` — click on N | Jump to N lines below current position |
| 1.7.5 | Variable in subroutine (public by default) | Resolve across visible scope |
| 1.7.6 | Variable in procedure (private by default) | Resolve only within procedure |
| 1.7.7 | `write` argument / `print_argument` with tag reference | Jump to tag |

### 1.8 Edge Cases

| # | Scenario | Expected |
|---|----------|----------|
| 1.8.1 | Click on a keyword (not identifier) | Return None |
| 1.8.2 | Click on whitespace | Return None (no named descendant) |
| 1.8.3 | Click on undefined symbol | Return None gracefully (no crash) |
| 1.8.4 | Document has parse errors — click on valid node | Still resolves definitions |
| 1.8.5 | Empty file | Return None |
| 1.8.6 | File with only a routine header and no body | Return None |

---

## 2. Goto Implementation

### 2.1 Method Overrides

| # | Scenario | Expected |
|---|----------|----------|
| 2.1.1 | Click method def that is overridden in 2 subclasses | Array with both subclass locations |
| 2.1.2 | Click method def with no overrides | None + warning message |
| 2.1.3 | Click method overridden in 3+ subclasses (deep hierarchy) | Array with all override locations |
| 2.1.4 | Private method override is included | Override appears in results regardless of visibility |
| 2.1.5 | Click method in class method call (`##class(X).Y()`) | Find overrides of Y across subclasses of X |

### 2.2 Class Implementations

| # | Scenario | Expected |
|---|----------|----------|
| 2.2.1 | Click class name in `class_definition` node | All direct subclasses |
| 2.2.2 | Click class name that is referenced (not in `class_definition`) | Subclasses of that referenced class |
| 2.2.3 | Class with no subclasses | None |

### 2.3 Edge Cases

| # | Scenario | Expected |
|---|----------|----------|
| 2.3.1 | Click on non-identifier node | Return None |
| 2.3.2 | Class not found in workspace | Return None (no crash) |

---

## 3. Document Diagnostics

### 3.1 Syntax Error Detection

| # | Scenario | Expected |
|---|----------|----------|
| 3.1.1 | Clean `.cls` file with no errors | Zero diagnostics |
| 3.1.2 | `.cls` with syntax error (e.g., `set =`) | One or more ERROR diagnostics |
| 3.1.3 | Clean `.mac` routine | Zero diagnostics |
| 3.1.4 | `.mac` with multiple syntax errors | Multiple diagnostics returned |
| 3.1.5 | Error message includes the unexpected token text | Message contains the error text |

### 3.2 XML Diagnostics

| # | Scenario | Expected |
|---|----------|----------|
| 3.2.1 | XML with valid ObjectScript in CDATA `<Implementation>` block | Zero injected diagnostics |
| 3.2.2 | XML with syntax errors in CDATA `<Implementation>` block | Injected ObjectScript diagnostics returned |
| 3.2.3 | XML host-level syntax error (malformed XML) | XML syntax error diagnostic |
| 3.2.4 | XML with empty `<Implementation>` block | Zero diagnostics for that block |
| 3.2.5 | XML with fake/malformed CDATA markers | Still detects injection ranges and reports errors |

### 3.3 Diagnostic Consistency

| # | Scenario | Expected |
|---|----------|----------|
| 3.3.1 | Request diagnostics for document not in any project | Return empty report (not crash) |
| 3.3.2 | Request diagnostics for document not yet tracked | Return empty report |

---

## 4. Code Actions (Refactoring)

### 4.1 Document Refactoring

| # | Scenario | Expected |
|---|----------|----------|
| 4.1.1 | `.mac` file — all 4 refactor levels offered (Do, Conditionals, For, All) | Actions list contains all 4 |
| 4.1.2 | `.cls` file — only 3 refactor levels (Conditionals, For, All) | No "Do" action for class files |
| 4.1.3 | `.xml` file — no document refactor levels | Empty actions or None |
| 4.1.4 | Document has parse errors | No document-level refactor actions offered |
| 4.1.5 | Refactor produces no changes (already modern syntax) | `build_document_refactor_edit` returns None |

### 4.2 Workspace Refactoring

| # | Scenario | Expected |
|---|----------|----------|
| 4.2.1 | Workspace refactor finds changes across multiple files | HashMap with multiple URLs |
| 4.2.2 | Workspace refactor with no legacy syntax anywhere | Empty changes |
| 4.2.3 | Trigger kind is AUTOMATIC | No workspace refactors in response |
| 4.2.4 | XML files are excluded from workspace refactor | Only `.cls`/`.mac` files in changes |

### 4.3 Execute Command

| # | Scenario | Expected |
|---|----------|----------|
| 4.3.1 | `objectscript.refactorDocument` with valid args | Edit applied |
| 4.3.2 | `objectscript.refactorWorkspace` with valid args | Edit applied |
| 4.3.3 | Legacy command `objectscript.refactorWorkspaceDottedDo` | Treated as DoCommands level |
| 4.3.4 | Missing URI argument | Logs error, returns None |
| 4.3.5 | Missing/invalid refactor level argument | Logs error, returns None |

---

## 5. Document Sync (didOpen / didChange)

### 5.1 didOpen

| # | Scenario | Expected |
|---|----------|----------|
| 5.1.1 | Open `.cls` file | Document tracked with correct FileType, class_id, class_name populated |
| 5.1.2 | Open `.mac` file | Document tracked as Routine, class_name = routine name |
| 5.1.3 | Open `.xml` file | Document tracked with FileType::Xml, no class_id/class_name |
| 5.1.4 | Open unsupported extension (`.txt`) | Ignored (no tracking) |

### 5.2 didChange — Incremental

| # | Scenario | Expected |
|---|----------|----------|
| 5.2.1 | Single ranged edit in middle of file | Content updated, tree re-parsed incrementally |
| 5.2.2 | Multiple sequential ranged edits in one notification | All edits applied in order |
| 5.2.3 | Full-text replacement (range = None) | Old tree discarded, full reparse |
| 5.2.4 | After edit, diagnostics reflect new content | New errors appear / old errors gone |
| 5.2.5 | After edit, goto_definition reflects new content | Definitions from updated code |
| 5.2.6 | Version is older than current | Warning logged (but still processed) |

### 5.3 didChangeWatchedFiles

| # | Scenario | Expected |
|---|----------|----------|
| 5.3.1 | New file created on disk | File gets indexed (handle_document_opened) |
| 5.3.2 | Existing file modified on disk | Re-indexed with new content |
| 5.3.3 | File deleted | Skipped (no action) |
| 5.3.4 | Non-ObjectScript file changes | Skipped |

---

## 6. Ordering & Timing

### 6.1 Index Order Independence

| # | Scenario | Expected |
|---|----------|----------|
| 6.1.1 | Subclass opened/indexed BEFORE superclass | After superclass loads, goto_def resolves to superclass |
| 6.1.2 | Superclass opened first, then subclass | Normal resolution works |
| 6.1.3 | File re-opened after disk modification (via watcher) | State reflects latest content |
| 6.1.4 | Same class opened twice (duplicate) | Second open is idempotent or updates version |

### 6.2 Cross-File Consistency

| # | Scenario | Expected |
|---|----------|----------|
| 6.2.1 | Class referenced in method call isn't in workspace | Returns None (not crash) |
| 6.2.2 | Inheritance chain partially loaded | Available links resolve; missing links return None |
| 6.2.3 | update_document triggers rebuild of inheritance/variables | Overrides and variables are current |

---

## 7. Capability Negotiation

| # | Scenario | Expected |
|---|----------|----------|
| 7.1 | Config with `enable_formatting: true` | `document_formatting_provider` = Some |
| 7.2 | Config with `enable_formatting: false` (default) | `document_formatting_provider` = None |
| 7.3 | Server always advertises definition, implementation, diagnostics | Capabilities present |

---

## Test Fixture Directories

```
objectscript-tests/
├── diagnostics/          — syntax error detection tests
├── gotodef/
│   ├── class-method-call/ — ##class(X).Y() resolution
│   ├── multiple-inheritance/ — left/right inheritance
│   ├── oref-contexts/    — oref in do_parameter, job_argument
│   └── routines/         — tag calls, offsets, cross-routine refs
├── implementation/       — override resolution with deep hierarchy
├── inheritance/          — class keyword inheritance
├── navigation/implementation/ — public/private overrides
├── ordering/             — index order independence tests
├── variables/            — public/private variable scoping
├── dependencies/         — method call dependency tracking
├── routines/             — refactoring tests
├── nested_dots/          — nested dotted statement tests
├── dotted-block/         — dotted block refactoring
└── local/                — large dotted statement tests
```
