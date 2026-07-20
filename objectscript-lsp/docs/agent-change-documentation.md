# Agent Change Documentation: Comprehensive LSP Test Suite

## 1. Problem

### Original issue

The ObjectScript LSP server had minimal test coverage. The existing `test-suite.md` only documented goto-definition cases for classes with incomplete structure, and the Rust test file (`src/test.rs`) had only 14 test functions — mostly covering variables, inheritance keywords, refactoring, and a single goto-implementation test. Major LSP features were untested or under-tested:

- No diagnostic tests (syntax error detection, XML injection)
- No class method call goto-definition tests
- No oref resolution tests across different AST contexts (do_parameter, job_argument)
- No multiple inheritance direction tests
- No deep hierarchy override tests
- No ordering/timing tests (child opened before parent)
- No edge case tests (undefined symbols, missing classes)
- No didOpen/didChange document sync tests
- No refactoring edge case tests

### Evidence

- User request: "I need to fully test all the current language server features (gotodef, goto_implementation, document diagnostics) as well as make sure they work consistently even with different ordering of indexing or opening of files"
- Existing `objectscript-tests/test-suite.md` was 124 lines, incomplete, and class-focused only
- Existing `src/test.rs` had 14 test functions total

### Scope

In scope:

- All LSP features currently implemented: goto_definition, goto_implementation, diagnostics, code actions/refactoring, document sync
- Test fixture creation for all scenarios
- Comprehensive test-suite.md documentation with numbered test cases
- Rust integration tests exercising the `ProjectData` and `ProjectState` APIs directly
- Ordering/timing scenarios (open order independence)
- Edge cases (undefined symbols, missing classes, empty results)

Out of scope:

- Fixing the compilation error in `lsp.rs` (match syntax error at line 794) — user stated they already fixed it
- Adding new LSP features
- Performance testing
- End-to-end LSP protocol testing via JSON-RPC

---

## 2. Solution

### Summary

Created 26 new test fixture files across 7 new directories, rewrote `objectscript-tests/test-suite.md` as a structured 264-line test plan with 7 sections and numbered cases, and added 30 new Rust test functions to `src/test.rs` covering goto-definition, goto-implementation, diagnostics, document sync, ordering, and refactoring.

### Technical approach

1. **Test fixtures**: Created minimal ObjectScript `.cls`, `.mac`, and `.xml` files that exercise specific LSP behaviors in isolation. Each fixture directory maps to a section in the test suite.

2. **Test-suite.md**: Restructured as a formal test plan with numbered cases in tables, organized by feature (Goto Definition, Goto Implementation, Diagnostics, Code Actions, Document Sync, Ordering, Capabilities).

3. **Rust tests**: Added tests that follow the existing pattern — use `setup_backend_and_workspace()` or `ProjectState::new()` to create state, call `handle_document_opened()` for ordering tests, and assert on `get_method_definition()`, `get_method_overrides()`, `get_class_implementations()`, `get_variable_definition()`, `get_class_definition()`, `get_oref_method_definition()`, and `collect_error_nodes()`.

### Design decisions

- **Test at the ProjectData/ProjectState API level, not the LSP handler level.**
  - Reason: The existing tests use this pattern. It avoids needing a mock LSP Client while still exercising the core logic.
  - Alternatives considered: Full LSP protocol tests via `LspService::build()` (only used in `src/lsp.rs` tests for the diagnostic handler).
  - Trade-off: Does not test the node-type dispatch logic in `goto_definition()` handler directly, but exercises the underlying resolution logic.

- **One fixture directory per test category.**
  - Reason: Keeps workspace indexing fast and isolated. Each test indexes only the files it needs.
  - Trade-off: Some fixture duplication (e.g., multiple "target" classes) but avoids cross-test interference.

- **Used `ProjectState::new()` + `handle_document_opened()` for ordering tests instead of `setup_backend_and_workspace()`.**
  - Reason: Ordering tests need to control the exact sequence of file loading. `setup_backend_and_workspace()` indexes everything at once.

### Important behavior changes

- No runtime behavior changes. All changes are test infrastructure.
- Removed debug `eprintln!` statements from one existing test (`test_routine_goto_definition_variable`).

---

## 3. Reconstruction Guide

### Base repository state

- Branch: `v1.5.0`
- Base commit: `dac4873bc0e639f12ee353fa42b60df09bb571d5`
- Comparison target: `HEAD` (unstaged working tree changes)

### Prerequisites

- Rust toolchain (edition 2024 support required)
- All crate dependencies already in `Cargo.lock`

### Step-by-step recreation

1. Create 7 new directories under `objectscript-tests/`:
   ```
   diagnostics/
   gotodef/class-method-call/
   gotodef/multiple-inheritance/
   gotodef/oref-contexts/
   gotodef/routines/
   implementation/
   ordering/
   ```

2. Create 26 fixture files (detailed below in Section 4).

3. Replace `objectscript-tests/test-suite.md` with the comprehensive 264-line test plan.

4. Modify `src/test.rs`:
   - Remove debug `eprintln!` statements from `test_routine_goto_definition_variable`
   - Add 30 new test functions after the existing test, organized by feature section

5. Verify compilation: `cargo test --no-run`

### Exact patch

The full diff for `src/test.rs` is 899 lines. Due to size, it was saved during evidence collection at:
`/Users/hkimura/.claude/projects/-Users-hkimura-zed-objectscript-objectscript-lsp-crates-objectscript-core/2d445cc9-da63-4802-9450-fed4010773f7/tool-results/bk4bjaexz.txt`

Regenerate with:
```bash
cd objectscript-lsp && git diff HEAD -- src/test.rs
```

---

## 4. ALL Changes

### Change manifest

| # | File | Status | Category | Why changed | How to recreate |
|---|------|--------|----------|-------------|-----------------|
| 1 | `objectscript-tests/test-suite.md` | Untracked (new) | Docs | Complete rewrite of test plan | Write 264-line structured test plan |
| 2 | `src/test.rs` | Modified | Test | Add 30 new integration tests | Append test functions after existing tests |
| 3 | `objectscript-tests/diagnostics/clean.cls` | Untracked (new) | Test | Fixture: clean class with no parse errors | Write minimal valid class |
| 4 | `objectscript-tests/diagnostics/syntax-error.cls` | Untracked (new) | Test | Fixture: class with intentional syntax error (`set =`) | Write class with malformed set command |
| 5 | `objectscript-tests/diagnostics/clean.mac` | Untracked (new) | Test | Fixture: clean routine with no parse errors | Write minimal valid routine |
| 6 | `objectscript-tests/diagnostics/multiple-errors.mac` | Untracked (new) | Test | Fixture: routine with 2+ syntax errors | Write routine with `set =` and unclosed string |
| 7 | `objectscript-tests/diagnostics/injected-clean.xml` | Untracked (new) | Test | Fixture: XML with valid ObjectScript in CDATA | Write XML export with valid `set x = 1` |
| 8 | `objectscript-tests/diagnostics/injected-error.xml` | Untracked (new) | Test | Fixture: XML with ObjectScript errors in CDATA | Write XML export with `set =` error |
| 9 | `objectscript-tests/gotodef/class-method-call/caller.cls` | Untracked (new) | Test | Fixture: class that calls `##class(Demo.Utility).Helper()` | Write class with class method call |
| 10 | `objectscript-tests/gotodef/class-method-call/utility.cls` | Untracked (new) | Test | Fixture: target class with Helper/Compute methods | Write utility class with two classmethods |
| 11 | `objectscript-tests/gotodef/multiple-inheritance/base.cls` | Untracked (new) | Test | Fixture: base class with Shared/Common methods | Write root class |
| 12 | `objectscript-tests/gotodef/multiple-inheritance/left-parent.cls` | Untracked (new) | Test | Fixture: left parent overriding base | Write class extending Demo.Base |
| 13 | `objectscript-tests/gotodef/multiple-inheritance/right-parent.cls` | Untracked (new) | Test | Fixture: right parent overriding base | Write class extending Demo.Base |
| 14 | `objectscript-tests/gotodef/multiple-inheritance/child-default.cls` | Untracked (new) | Test | Fixture: child with default (left) inheritance | Write class extending both parents |
| 15 | `objectscript-tests/gotodef/multiple-inheritance/child-right.cls` | Untracked (new) | Test | Fixture: child with `[Inheritance = right]` | Write class with right inheritance keyword |
| 16 | `objectscript-tests/gotodef/oref-contexts/oref-do-parameter.cls` | Untracked (new) | Test | Fixture: oref method call in `do obj.Run()` | Write class with do_parameter oref |
| 17 | `objectscript-tests/gotodef/oref-contexts/oref-job-argument.cls` | Untracked (new) | Test | Fixture: oref method call in `job worker.Execute()` | Write class with job_argument oref |
| 18 | `objectscript-tests/gotodef/oref-contexts/target.cls` | Untracked (new) | Test | Fixture: target class for oref resolution | Write Demo.Target with %New/Run/Execute |
| 19 | `objectscript-tests/gotodef/routines/tag-calls.mac` | Untracked (new) | Test | Fixture: routine with tag calls (do/goto) | Write routine with helper/finish labels |
| 20 | `objectscript-tests/gotodef/routines/cross-routine-ref.mac` | Untracked (new) | Test | Fixture: cross-routine reference `do helper^tagcalls` | Write short routine referencing tagcalls |
| 21 | `objectscript-tests/gotodef/routines/offset-goto.mac` | Untracked (new) | Test | Fixture: numeric offset `do main+2` | Write routine with offset reference |
| 22 | `objectscript-tests/implementation/no-overrides.cls` | Untracked (new) | Test | Fixture: class with method that has no overrides | Write standalone class |
| 23 | `objectscript-tests/implementation/deep-super.cls` | Untracked (new) | Test | Fixture: top of 3-level hierarchy | Write base class with DeepMethod |
| 24 | `objectscript-tests/implementation/deep-mid.cls` | Untracked (new) | Test | Fixture: middle of hierarchy | Write class extending Demo.DeepSuper |
| 25 | `objectscript-tests/implementation/deep-leaf-one.cls` | Untracked (new) | Test | Fixture: leaf override #1 | Write class extending Demo.DeepMid |
| 26 | `objectscript-tests/implementation/deep-leaf-two.cls` | Untracked (new) | Test | Fixture: leaf override #2 | Write class extending Demo.DeepMid |
| 27 | `objectscript-tests/ordering/parent.cls` | Untracked (new) | Test | Fixture: parent class for ordering tests | Write Demo.OrderParent with Greet/Work |
| 28 | `objectscript-tests/ordering/child.cls` | Untracked (new) | Test | Fixture: child class for ordering tests | Write Demo.OrderChild extending parent |

### Detailed file changes

#### `src/test.rs`

- Status: Modified, unstaged
- Category: Test
- Change type: Feature (new tests)
- Problem addressed: Minimal test coverage for LSP features
- Solution implemented: Added 30 new test functions covering 6 feature areas
- Exact changes:
  - Removed 4 `eprintln!` debug lines and 1 unused variable from `test_routine_goto_definition_variable`
  - Added test section: **GOTO DEFINITION — CLASS METHOD CALLS** (3 tests)
    - `test_goto_def_class_method_call_resolves_method`
    - `test_goto_def_class_method_call_nonexistent_returns_empty`
    - `test_goto_def_class_reference_resolves_to_class`
  - Added test section: **GOTO DEFINITION — OREF CONTEXTS** (2 tests)
    - `test_goto_def_oref_resolves_method_in_target_class`
    - `test_goto_def_oref_job_argument_resolves_method`
  - Added test section: **GOTO DEFINITION — MULTIPLE INHERITANCE** (2 tests)
    - `test_goto_def_multiple_inheritance_default_left`
    - `test_goto_def_multiple_inheritance_right_direction`
  - Added test section: **GOTO IMPLEMENTATION — DEEP HIERARCHY** (5 tests)
    - `test_goto_implementation_deep_hierarchy_from_super`
    - `test_goto_implementation_deep_hierarchy_from_mid`
    - `test_goto_implementation_no_overrides_returns_empty`
    - `test_goto_implementation_class_subclasses`
    - `test_goto_implementation_class_with_no_subclasses`
  - Added test section: **DIAGNOSTICS** (6 tests)
    - `test_diagnostics_clean_cls_has_no_errors`
    - `test_diagnostics_syntax_error_cls_has_errors`
    - `test_diagnostics_clean_routine_has_no_errors`
    - `test_diagnostics_multiple_errors_routine`
    - `test_diagnostics_xml_injected_clean_has_no_errors`
    - `test_diagnostics_xml_injected_error_has_errors`
  - Added test section: **ORDERING / TIMING** (4 tests)
    - `test_ordering_child_opened_before_parent`
    - `test_ordering_parent_opened_before_child`
    - `test_ordering_missing_class_reference_returns_empty`
    - `test_ordering_duplicate_open_is_idempotent`
  - Added test section: **GOTO DEFINITION — EDGE CASES** (2 tests)
    - `test_goto_def_undefined_symbol_returns_empty`
    - `test_goto_def_nonexistent_class_returns_empty`
  - Added test section: **DOCUMENT SYNC — didOpen** (3 tests)
    - `test_did_open_cls_populates_class_id_and_name`
    - `test_did_open_routine_populates_class_name_as_routine`
    - `test_did_open_xml_no_class_id`
  - Added test section: **DOCUMENT SYNC — update_document** (1 test)
    - `test_update_document_rebuilds_semantics`
  - Added test section: **REFACTORING** (2 tests)
    - `test_refactor_no_changes_returns_none`
    - `test_refactor_workspace_excludes_xml`
- Side effects: None (test-only changes)
- Verification: Not run (compilation check was interrupted by user)
- Notes: XML diagnostic tests use `ProjectState` to parse XML internally (avoiding direct `tree_sitter_xml` dependency which is only in `objectscript-core`)

#### `objectscript-tests/test-suite.md`

- Status: Untracked (new file, complete rewrite replacing the old tracked version)
- Category: Docs
- Change type: Documentation
- Problem addressed: Old test-suite.md was 124 lines, incomplete, unstructured
- Solution implemented: 264-line structured test plan with 7 sections, numbered test cases in tables, fixture directory listing
- Exact changes:
  - Section 1: Goto Definition (8 subsections, ~40 test cases)
  - Section 2: Goto Implementation (3 subsections, ~10 test cases)
  - Section 3: Document Diagnostics (3 subsections, ~12 test cases)
  - Section 4: Code Actions/Refactoring (3 subsections, ~12 test cases)
  - Section 5: Document Sync (3 subsections, ~12 test cases)
  - Section 6: Ordering & Timing (2 subsections, ~7 test cases)
  - Section 7: Capability Negotiation (3 test cases)
  - Fixture directory tree reference
- Side effects: None
- Verification: Visual review

#### `objectscript-tests/diagnostics/clean.cls`

- Status: Untracked (new)
- Category: Test fixture
- Change type: New fixture
- Problem addressed: Need a valid .cls file that produces zero parse errors
- Solution implemented: Minimal `Class Demo.Clean` with one valid `ClassMethod`
- Exact changes: 10-line file with `Set x = 1`, `Write x`, `Quit 1`
- Verification: Used in `test_diagnostics_clean_cls_has_no_errors`

#### `objectscript-tests/diagnostics/syntax-error.cls`

- Status: Untracked (new)
- Category: Test fixture
- Change type: New fixture
- Problem addressed: Need a .cls file that intentionally produces a parse error
- Solution implemented: `Set x =` (missing RHS) inside a ClassMethod
- Exact changes: 10-line file with intentionally broken `Set` command
- Verification: Used in `test_diagnostics_syntax_error_cls_has_errors`

#### `objectscript-tests/diagnostics/clean.mac`

- Status: Untracked (new)
- Category: Test fixture
- Change type: New fixture
- Problem addressed: Need a valid routine file for zero-error diagnostic test
- Solution implemented: `ROUTINE clean` with `set x = 1` / `w x` / `quit`
- Verification: Used in `test_diagnostics_clean_routine_has_no_errors`

#### `objectscript-tests/diagnostics/multiple-errors.mac`

- Status: Untracked (new)
- Category: Test fixture
- Change type: New fixture
- Problem addressed: Need a routine with 2+ distinct parse errors
- Solution implemented: `set =` (missing LHS) and unclosed string literal `"hello`
- Verification: Used in `test_diagnostics_multiple_errors_routine`

#### `objectscript-tests/diagnostics/injected-clean.xml`

- Status: Untracked (new)
- Category: Test fixture
- Change type: New fixture
- Problem addressed: Need XML with valid ObjectScript in CDATA to verify zero injected errors
- Solution implemented: XML Export with `<Implementation><![CDATA[ set x = 1 ... ]]></Implementation>`
- Verification: Used in `test_diagnostics_xml_injected_clean_has_no_errors`

#### `objectscript-tests/diagnostics/injected-error.xml`

- Status: Untracked (new)
- Category: Test fixture
- Change type: New fixture
- Problem addressed: Need XML with ObjectScript errors in CDATA to verify error detection
- Solution implemented: XML Export with `set =` and unclosed string in CDATA block
- Verification: Used in `test_diagnostics_xml_injected_error_has_errors`

#### `objectscript-tests/gotodef/class-method-call/caller.cls`

- Status: Untracked (new)
- Category: Test fixture
- Change type: New fixture
- Problem addressed: Need a class that calls `##class(Demo.Utility).Helper()`
- Solution implemented: `Class Demo.Caller` with ClassMethod that calls Demo.Utility methods
- Verification: Used in `test_goto_def_class_method_call_resolves_method`

#### `objectscript-tests/gotodef/class-method-call/utility.cls`

- Status: Untracked (new)
- Category: Test fixture
- Change type: New fixture
- Problem addressed: Target class for class method call resolution
- Solution implemented: `Class Demo.Utility` with `Helper()` and `Compute()` classmethods
- Verification: Used in class method call tests

#### `objectscript-tests/gotodef/multiple-inheritance/base.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Root of multi-inheritance test hierarchy
- Solution implemented: `Class Demo.Base` with `Shared()` and `Common()` methods
- Verification: Used in multiple inheritance tests

#### `objectscript-tests/gotodef/multiple-inheritance/left-parent.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Left parent in multi-inheritance chain
- Solution implemented: `Class Demo.LeftParent Extends Demo.Base` overriding both methods
- Verification: Used in multiple inheritance tests

#### `objectscript-tests/gotodef/multiple-inheritance/right-parent.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Right parent in multi-inheritance chain
- Solution implemented: `Class Demo.RightParent Extends Demo.Base` overriding both methods
- Verification: Used in multiple inheritance tests

#### `objectscript-tests/gotodef/multiple-inheritance/child-default.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Child using default (left) inheritance direction
- Solution implemented: `Class Demo.ChildDefault Extends (Demo.LeftParent, Demo.RightParent)`
- Verification: Used in `test_goto_def_multiple_inheritance_default_left`

#### `objectscript-tests/gotodef/multiple-inheritance/child-right.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Child using explicit right inheritance direction
- Solution implemented: `Class Demo.ChildRight Extends (...) [Inheritance = right]`
- Verification: Used in `test_goto_def_multiple_inheritance_right_direction`

#### `objectscript-tests/gotodef/oref-contexts/oref-do-parameter.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Oref method call in `do obj.Run()` context
- Solution implemented: `Class Demo.OrefDoParam` with `[ProcedureBlock = 0]` method doing `do obj.Run()`
- Verification: Used in `test_goto_def_oref_resolves_method_in_target_class`

#### `objectscript-tests/gotodef/oref-contexts/oref-job-argument.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Oref method call in `job worker.Execute()` context
- Solution implemented: `Class Demo.OrefJobArg` with `job worker.Execute()` statement
- Verification: Used in `test_goto_def_oref_job_argument_resolves_method`

#### `objectscript-tests/gotodef/oref-contexts/target.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Target class that oref variables resolve to
- Solution implemented: `Class Demo.Target` with `%New()`, `Run()`, `Execute()` methods
- Verification: Used in both oref context tests

#### `objectscript-tests/gotodef/routines/tag-calls.mac`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Routine with labels for tag call / goto resolution
- Solution implemented: `ROUTINE tagcalls` with `main`, `helper`, `finish` labels and `do`/`goto` calls
- Verification: Available for routine goto-definition tests

#### `objectscript-tests/gotodef/routines/cross-routine-ref.mac`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Cross-routine reference (`do helper^tagcalls`)
- Solution implemented: `ROUTINE crossref` with `do helper^tagcalls`
- Verification: Available for cross-routine resolution tests

#### `objectscript-tests/gotodef/routines/offset-goto.mac`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Numeric offset goto (`do main+2`)
- Solution implemented: `ROUTINE offsetgoto` with `do main+2` targeting a specific line
- Verification: Available for numeric offset tests

#### `objectscript-tests/implementation/no-overrides.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Class with no subclasses for negative implementation test
- Solution implemented: `Class Demo.NoOverrides` with single `Unique()` method
- Verification: Used in `test_goto_implementation_no_overrides_returns_empty` and `test_goto_implementation_class_with_no_subclasses`

#### `objectscript-tests/implementation/deep-super.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Top of 3-level override hierarchy
- Solution implemented: `Class Demo.DeepSuper` with `DeepMethod()`
- Verification: Used in `test_goto_implementation_deep_hierarchy_from_super`

#### `objectscript-tests/implementation/deep-mid.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Middle of 3-level hierarchy
- Solution implemented: `Class Demo.DeepMid Extends Demo.DeepSuper` overriding `DeepMethod()`
- Verification: Used in deep hierarchy tests

#### `objectscript-tests/implementation/deep-leaf-one.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Leaf #1 of hierarchy
- Solution implemented: `Class Demo.DeepLeafOne Extends Demo.DeepMid` overriding `DeepMethod()`
- Verification: Used in `test_goto_implementation_deep_hierarchy_from_mid`

#### `objectscript-tests/implementation/deep-leaf-two.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Leaf #2 of hierarchy
- Solution implemented: `Class Demo.DeepLeafTwo Extends Demo.DeepMid` overriding `DeepMethod()`
- Verification: Used in `test_goto_implementation_deep_hierarchy_from_mid`

#### `objectscript-tests/ordering/parent.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Parent class for index-order tests
- Solution implemented: `Class Demo.OrderParent` with `Greet()` and `Work()` methods
- Verification: Used in ordering tests

#### `objectscript-tests/ordering/child.cls`

- Status: Untracked (new)
- Category: Test fixture
- Problem addressed: Child class for index-order tests
- Solution implemented: `Class Demo.OrderChild Extends Demo.OrderParent` overriding `Greet()`
- Verification: Used in `test_ordering_child_opened_before_parent` and `test_ordering_parent_opened_before_child`

---

## 5. Tests and Verification

### Commands run

| Command | Result | Evidence / Notes |
|---------|--------|------------------|
| `cargo test --no-run` | Not completed | User interrupted the compilation check |
| `cargo check` | Not run on final state | Was run earlier; had pre-existing errors in `lsp.rs` which user said they fixed |

### Automated tests

- Added (30 new test functions):
  - `test_goto_def_class_method_call_resolves_method` — verifies method_defs lookup for `Demo.Utility.Helper`
  - `test_goto_def_class_method_call_nonexistent_returns_empty` — verifies missing method returns None
  - `test_goto_def_class_reference_resolves_to_class` — verifies `get_class_definition` for Demo.Utility
  - `test_goto_def_oref_resolves_method_in_target_class` — verifies oref resolution in do_parameter
  - `test_goto_def_oref_job_argument_resolves_method` — verifies oref resolution in job_argument
  - `test_goto_def_multiple_inheritance_default_left` — verifies inheritance direction defaults
  - `test_goto_def_multiple_inheritance_right_direction` — verifies `[Inheritance = right]` keyword
  - `test_goto_implementation_deep_hierarchy_from_super` — verifies overrides of top-level method
  - `test_goto_implementation_deep_hierarchy_from_mid` — verifies overrides of mid-level method
  - `test_goto_implementation_no_overrides_returns_empty` — verifies empty result for leaf method
  - `test_goto_implementation_class_subclasses` — verifies `get_class_implementations`
  - `test_goto_implementation_class_with_no_subclasses` — verifies empty for standalone class
  - `test_diagnostics_clean_cls_has_no_errors` — verifies zero ERROR nodes for valid .cls
  - `test_diagnostics_syntax_error_cls_has_errors` — verifies ERROR nodes for broken .cls
  - `test_diagnostics_clean_routine_has_no_errors` — verifies zero ERROR nodes for valid .mac
  - `test_diagnostics_multiple_errors_routine` — verifies 2+ ERROR nodes for broken .mac
  - `test_diagnostics_xml_injected_clean_has_no_errors` — verifies zero injected errors for valid XML
  - `test_diagnostics_xml_injected_error_has_errors` — verifies injected errors for broken XML
  - `test_ordering_child_opened_before_parent` — verifies late-binding resolution
  - `test_ordering_parent_opened_before_child` — verifies normal order resolution
  - `test_ordering_missing_class_reference_returns_empty` — verifies no crash on missing class
  - `test_ordering_duplicate_open_is_idempotent` — verifies version update on re-open
  - `test_goto_def_undefined_symbol_returns_empty` — verifies empty for nonexistent variable
  - `test_goto_def_nonexistent_class_returns_empty` — verifies empty for missing class
  - `test_did_open_cls_populates_class_id_and_name` — verifies .cls document tracking
  - `test_did_open_routine_populates_class_name_as_routine` — verifies .mac document tracking
  - `test_did_open_xml_no_class_id` — verifies .xml has no class_id
  - `test_update_document_rebuilds_semantics` — verifies semantics survive update
  - `test_refactor_no_changes_returns_none` — verifies None for modern syntax
  - `test_refactor_workspace_excludes_xml` — verifies XML excluded from refactor

### Manual verification

- None performed (test-only changes, compilation was interrupted)

### Not verified

- Full compilation of `src/test.rs` against the crate — user interrupted `cargo test --no-run`
- Actual test execution — no `cargo test` was run
- XML diagnostic tests depend on `tree_sitter_objectscript_playground::LANGUAGE_OBJECTSCRIPT` being available (it is in workspace deps)
- The `tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL` import in diagnostic tests requires the `tree-sitter-objectscript` crate (in workspace deps)

---

## 6. Dependencies, Config, and Environment Changes

### Dependencies

No dependency changes were made as part of this test suite work.

### Configuration

No configuration changes were made.

### Environment assumptions

No new environment assumptions were introduced.

---

## 7. Generated Files, Build Artifacts, and Lockfiles

No generated files, build artifacts, or lockfiles changed.

---

## 8. Risks and Follow-ups

### Risks

- **Compilation not verified on final state**: The `cargo test --no-run` was interrupted. The tests may have minor compilation issues (e.g., the `tree_sitter_objectscript` import for `.cls` diagnostics tests).
- **Oref tests may fail if `get_oref_method_definition` requires variables to be in the dependency graph**: The oref fixtures use `[ProcedureBlock = 0]` to make variables public, but the oref lookup may need the full call graph to resolve `obj` → `Demo.Target`.
- **Some fixture files may parse differently than expected**: The tree-sitter grammars for ObjectScript have specific node kinds; if the fixture syntax doesn't produce the expected AST, tests may fail.

### Follow-up work

- Run `cargo test` and fix any compilation or assertion failures
- Add tests for:
  - `did_change` incremental edit behavior
  - `did_change_watched_files` file watcher behavior
  - Code action response structure (what actions are offered for each file type)
  - Execute command flow (document refactor vs workspace refactor)
  - Capability negotiation (formatting flag)
- Consider adding the routine tag-call fixture tests (fixtures exist but no Rust test exercises them yet at the handler level)
- Consider end-to-end LSP protocol tests for `goto_definition` handler dispatch logic

### Rollback plan

1. Delete all untracked files: `git clean -fd objectscript-tests/diagnostics objectscript-tests/gotodef/class-method-call objectscript-tests/gotodef/multiple-inheritance objectscript-tests/gotodef/oref-contexts objectscript-tests/gotodef/routines objectscript-tests/implementation objectscript-tests/ordering`
2. Restore `src/test.rs`: `git checkout -- src/test.rs`
3. Restore `objectscript-tests/test-suite.md`: `git checkout -- objectscript-tests/test-suite.md`
4. Run `cargo test` to confirm previous state works

---

## 9. Completeness Checklist

- [x] Every file from `git status --short` is documented (modified files noted as pre-existing changes not part of this work, except `src/test.rs` and `test-suite.md`)
- [x] Every staged file is documented (none staged)
- [x] Every unstaged file is documented (`src/test.rs`)
- [x] Every untracked file is documented (26 fixtures + test-suite.md = 27 files)
- [x] Every deleted file is documented (none deleted)
- [x] Every renamed file is documented (none renamed)
- [x] Every generated file is documented (none generated)
- [x] Every lockfile is documented (none changed)
- [x] Every dependency change is documented (none)
- [x] Every config change is documented (none)
- [x] Every test change is documented (30 new tests)
- [x] Every documentation change is documented (test-suite.md rewrite)
- [x] Verification commands are listed with results
- [x] Unverified areas are explicitly listed
- [x] The reconstruction guide is complete enough to duplicate the implementation

---

## Appendix A: Full Unified Diff

The full patch for `src/test.rs` (899 lines) was saved during evidence collection. Regenerate with:

```bash
cd objectscript-lsp && git diff HEAD -- src/test.rs
```

For the complete working tree diff:

```bash
cd objectscript-lsp && git diff HEAD
```

---

## Appendix B: Git Status and File Inventory

### `git status --short` (test-relevant files only)

```text
 M src/test.rs
?? objectscript-tests/diagnostics/
?? objectscript-tests/gotodef/class-method-call/
?? objectscript-tests/gotodef/multiple-inheritance/
?? objectscript-tests/gotodef/oref-contexts/
?? objectscript-tests/gotodef/routines/
?? objectscript-tests/implementation/
?? objectscript-tests/ordering/
?? objectscript-tests/test-suite.md
```

### Untracked files (26 fixture files + 1 doc)

```text
objectscript-tests/diagnostics/clean.cls
objectscript-tests/diagnostics/clean.mac
objectscript-tests/diagnostics/injected-clean.xml
objectscript-tests/diagnostics/injected-error.xml
objectscript-tests/diagnostics/multiple-errors.mac
objectscript-tests/diagnostics/syntax-error.cls
objectscript-tests/gotodef/class-method-call/caller.cls
objectscript-tests/gotodef/class-method-call/utility.cls
objectscript-tests/gotodef/multiple-inheritance/base.cls
objectscript-tests/gotodef/multiple-inheritance/child-default.cls
objectscript-tests/gotodef/multiple-inheritance/child-right.cls
objectscript-tests/gotodef/multiple-inheritance/left-parent.cls
objectscript-tests/gotodef/multiple-inheritance/right-parent.cls
objectscript-tests/gotodef/oref-contexts/oref-do-parameter.cls
objectscript-tests/gotodef/oref-contexts/oref-job-argument.cls
objectscript-tests/gotodef/oref-contexts/target.cls
objectscript-tests/gotodef/routines/cross-routine-ref.mac
objectscript-tests/gotodef/routines/offset-goto.mac
objectscript-tests/gotodef/routines/tag-calls.mac
objectscript-tests/implementation/deep-leaf-one.cls
objectscript-tests/implementation/deep-leaf-two.cls
objectscript-tests/implementation/deep-mid.cls
objectscript-tests/implementation/deep-super.cls
objectscript-tests/implementation/no-overrides.cls
objectscript-tests/ordering/child.cls
objectscript-tests/ordering/parent.cls
objectscript-tests/test-suite.md
```

---

## Appendix C: New Test Functions Summary

| # | Function name | Type | Fixture directory |
|---|---------------|------|-------------------|
| 1 | `test_goto_def_class_method_call_resolves_method` | async | gotodef/class-method-call |
| 2 | `test_goto_def_class_method_call_nonexistent_returns_empty` | async | gotodef/class-method-call |
| 3 | `test_goto_def_class_reference_resolves_to_class` | async | gotodef/class-method-call |
| 4 | `test_goto_def_oref_resolves_method_in_target_class` | async | gotodef/oref-contexts |
| 5 | `test_goto_def_oref_job_argument_resolves_method` | async | gotodef/oref-contexts |
| 6 | `test_goto_def_multiple_inheritance_default_left` | async | gotodef/multiple-inheritance |
| 7 | `test_goto_def_multiple_inheritance_right_direction` | async | gotodef/multiple-inheritance |
| 8 | `test_goto_implementation_deep_hierarchy_from_super` | async | implementation |
| 9 | `test_goto_implementation_deep_hierarchy_from_mid` | async | implementation |
| 10 | `test_goto_implementation_no_overrides_returns_empty` | async | implementation |
| 11 | `test_goto_implementation_class_subclasses` | async | implementation |
| 12 | `test_goto_implementation_class_with_no_subclasses` | async | implementation |
| 13 | `test_diagnostics_clean_cls_has_no_errors` | sync | diagnostics |
| 14 | `test_diagnostics_syntax_error_cls_has_errors` | sync | diagnostics |
| 15 | `test_diagnostics_clean_routine_has_no_errors` | sync | diagnostics |
| 16 | `test_diagnostics_multiple_errors_routine` | sync | diagnostics |
| 17 | `test_diagnostics_xml_injected_clean_has_no_errors` | sync | diagnostics |
| 18 | `test_diagnostics_xml_injected_error_has_errors` | sync | diagnostics |
| 19 | `test_ordering_child_opened_before_parent` | async | ordering |
| 20 | `test_ordering_parent_opened_before_child` | async | ordering |
| 21 | `test_ordering_missing_class_reference_returns_empty` | async | ordering |
| 22 | `test_ordering_duplicate_open_is_idempotent` | async | ordering |
| 23 | `test_goto_def_undefined_symbol_returns_empty` | async | gotodef/class-method-call |
| 24 | `test_goto_def_nonexistent_class_returns_empty` | async | gotodef/class-method-call |
| 25 | `test_did_open_cls_populates_class_id_and_name` | sync | (inline content) |
| 26 | `test_did_open_routine_populates_class_name_as_routine` | sync | (inline content) |
| 27 | `test_did_open_xml_no_class_id` | sync | (inline content) |
| 28 | `test_update_document_rebuilds_semantics` | async | ordering |
| 29 | `test_refactor_no_changes_returns_none` | sync | (inline content) |
| 30 | `test_refactor_workspace_excludes_xml` | async | diagnostics |
