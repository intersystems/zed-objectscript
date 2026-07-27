# Goto-Implementation Feature

## Overview

The goto-implementation feature navigates from a class or method definition to its **subclass overrides**. This is the inverse of goto-definition's superclass navigation — where goto-def goes "up" the inheritance chain, goto-implementation goes "down" to find subclasses and overriding methods.

## Supported Symbols

### Classes

| Cursor position | Navigation target |
|---|---|
| Class name at its own definition (`Class MyClass`) | All subclasses that extend this class |
| Class name used as a reference (`##class(MyClass)`) | All subclasses that extend the referenced class |

### Methods

| Cursor position | Navigation target |
|---|---|
| Method name at its definition site (`Method Save()`) | All subclass methods that override this method |
| Method name in a class method call (`##class(Cls).Method()`) | All subclass methods that override the referenced method |

## Behavior

- If exactly one implementation is found, the editor navigates directly to it.
- If multiple implementations are found, the editor presents a picker with all locations.
- If no implementations are found, a warning message is shown: "No implementations were found for the given symbol."
