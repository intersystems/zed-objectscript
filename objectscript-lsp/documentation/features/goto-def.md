# Goto-Definition Feature

## Overview

The goto-definition feature resolves the definition site of a symbol under the cursor. It handles classes, methods (instance, class, procedures, subroutines, relative dot methods, and oref-chained), variables (private and public), superclass navigation, and numeric line offsets.

## Supported Symbols

### Classes

| Cursor position | Navigation target |
|---|---|
| Class name at its own definition (`Class MyClass Extends Super`) | The superclass(es) it extends |
| Class name used as a reference (`##class(Demo.Person)`) | The class definition file of that class name |
| Routine name in a line reference (`^RoutineName`) | The routine file |

### Methods

| Cursor position | Navigation target |
|---|---|
| Method name at its definition site (`Method Save()`) | The superclass method it overrides (if any) |
| Relative dot method call (`..MethodName()`) | The method definition in the same class |
| Class method call (`##class(Cls).MethodName()`) | The method definition in the referenced class |
| Same-file label call (`Do Label`, `Write Label`) | The label/subroutine definition in the current file |
| Cross-routine label call (`Do Label^Routine`, `$$Label^Routine`) | The label definition in the referenced routine |
| Label with offset (`Do Label+3^Routine`) | The method definition, offset by N lines |
| Oref method call, cursor on method (`obj.Method()`) | The method definition in the resolved class of `obj` |
| Oref method from `%New` (`##class(Cls).%New().Method()`) | The method definition in `Cls` |

### Variables

| Cursor position | Navigation target |
|---|---|
| Local variable reference | The closest preceding definition of that variable in the same scope (or parent scopes) |
| Public variable (non-ProcedureBlock or declared public) | The closest preceding definition, potentially across calling methods via the dependency graph |
| Oref variable, cursor on variable name (`obj` in `obj.Method()`) | The assignment site where the oref was created |
| Global variable (`^GlobalName`) | Same resolution as local variables (scope-based lookup) |

### Line Offsets

| Cursor position | Navigation target |
|---|---|
| Numeric literal in a tag call (`3` in `Do Label+3`) | The line N rows below the current position |

## Variable Resolution Rules

Variable resolution picks the **closest definition that appears before the reference**:

- If multiple `Set` commands assign the same variable in the same scope, goto-def resolves to the one nearest (but still before) the cursor.
- If no definition exists in the current scope, parent scopes and ancestor scopes are checked.
- For public variables, the DependencyGraph traces callers (via BFS) and finds the definition in the shallowest ancestor method that defines the variable before the call site.

**Example:**
```objectscript
Set x = 2
Set y = 3
Set x = 3
Write x   ; goto-def on x here → resolves to "Set x = 3"
```

## Oref Resolution Rules

When resolving `obj.Method()`, the LSP must determine what class `obj` is an instance of:

1. Finds the assignment of `obj` (e.g., `Set obj = ##class(Demo.Person).%New()`)
2. Extracts the class from the `%New()` call
3. Looks up `Method` in that class's method definitions

If the variable is public, the same DependencyGraph-based ancestor traversal is used to find the assignment across method boundaries.

## Scopes

Scopes that create boundaries for variable resolution:
- Classes
- Methods
- Subroutines / Procedures
- Conditional blocks (`If`/`ElseIf`/`Else`)

## Inheritance Rules for Superclass Navigation

When navigating from a method definition to its superclass override, or from a relative method call to an inherited method, the LSP uses the OverrideIndex. For multiple inheritance, the default precedence is left-to-right through the `Extends (...)` list. If the class declares `[Inheritance = right]`, the precedence is right-to-left.

Inherited members count when applying that precedence. For example, if `Demo.ChildDefault Extends (Demo.LeftParent, Demo.RightParent)`, `Demo.LeftParent Extends Demo.Base`, `Demo.Base` defines `Common`, and `Demo.RightParent` also defines `Common`, goto-definition from `Demo.ChildDefault` should resolve `Common` to `Demo.Base`. `Demo.LeftParent` comes first, so its effective inherited `Common` wins over `Demo.RightParent.Common`.

## TODO 
I still need to use `kill` and `new` statements in my analysis of what variable definitions are actually valid from a given method. If a `kill` statement appears, any definitions that came before that should be nullified.
