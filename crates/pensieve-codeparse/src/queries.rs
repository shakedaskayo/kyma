//! Tree-sitter query strings for import extraction — P1 (structural only).
//!
//! Each constant is an S-expression query that captures import paths.  The
//! capture name `@import` is used uniformly; the parse module collects every
//! matched capture with that name and records its start row as the line
//! number.

/// Rust: `use crate::foo::bar;` and `mod foo;` / `mod foo { … }`.
///
/// In tree-sitter-rust 0.24 the use-declaration children are not field-named
/// `argument` — the path node is a direct child.  We match all child patterns
/// directly without the `argument:` field prefix.
pub const RUST_IMPORTS: &str = r#"
(use_declaration
  (scoped_identifier) @import)

(use_declaration
  (scoped_use_list
    (scoped_identifier) @import))

(use_declaration
  (scoped_use_list
    (identifier) @import))

(use_declaration
  (use_wildcard
    (scoped_identifier) @import))

(use_declaration
  (use_as_clause
    (scoped_identifier) @import))

(use_declaration
  (identifier) @import)

(mod_item
  name: (identifier) @import)
"#;

/// Python: `import foo`, `from foo import bar`, `from .rel import baz`.
pub const PYTHON_IMPORTS: &str = r#"
(import_statement
  name: (dotted_name) @import)

(import_from_statement
  module_name: (dotted_name) @import)

(import_from_statement
  module_name: (relative_import) @import)
"#;

/// TypeScript / JavaScript: `import … from "path"`, `require("path")`.
pub const TS_JS_IMPORTS: &str = r#"
(import_statement
  source: (string) @import)

(call_expression
  function: (identifier) @_fn
  arguments: (arguments (string) @import)
  (#eq? @_fn "require"))

(export_statement
  source: (string) @import)
"#;

/// Go: `import "pkg"` and `import ( "pkg" )`.
pub const GO_IMPORTS: &str = r#"
(import_spec
  path: (interpreted_string_literal) @import)
"#;

// ── Definition queries (functions + classes) ────────────────────────────────
// Capture `@func` for callable definitions and `@class` for type definitions.
// The parse module records each captured name + its 1-based line.

/// Rust: functions, plus struct/enum/trait as "classes".
pub const RUST_DEFS: &str = r#"
(function_item name: (identifier) @func)
(function_signature_item name: (identifier) @func)
(struct_item name: (type_identifier) @class)
(enum_item name: (type_identifier) @class)
(trait_item name: (type_identifier) @class)
"#;

/// Python: functions/methods + classes.
pub const PYTHON_DEFS: &str = r#"
(function_definition name: (identifier) @func)
(class_definition name: (identifier) @class)
"#;

/// TypeScript / JavaScript: function declarations, methods, classes.
/// `(_)` for the class name handles both grammars (type_identifier vs identifier).
pub const TS_JS_DEFS: &str = r#"
(function_declaration name: (identifier) @func)
(method_definition name: (property_identifier) @func)
(class_declaration name: (_) @class)
"#;

/// Go: functions, methods (with receiver), and type specs as "classes".
pub const GO_DEFS: &str = r#"
(function_declaration name: (identifier) @func)
(method_declaration name: (field_identifier) @func)
(type_spec name: (type_identifier) @class)
"#;

// ── Call queries ────────────────────────────────────────────────────────────
// `@call_fn` captures a directly-called function name (`foo()`); `@call_recv`
// captures the receiver root of a method/attribute call (`boto3.client()` →
// `boto3`) so functions can be linked to the libraries/modules they use.

pub const RUST_CALLS: &str = r#"
(call_expression function: (identifier) @call_fn)
(call_expression function: (field_expression value: (identifier) @call_recv))
(call_expression function: (scoped_identifier path: (identifier) @call_recv))
"#;

pub const PYTHON_CALLS: &str = r#"
(call function: (identifier) @call_fn)
(call function: (attribute object: (identifier) @call_recv))
"#;

pub const TS_JS_CALLS: &str = r#"
(call_expression function: (identifier) @call_fn)
(call_expression function: (member_expression object: (identifier) @call_recv))
"#;

pub const GO_CALLS: &str = r#"
(call_expression function: (identifier) @call_fn)
(call_expression function: (selector_expression operand: (identifier) @call_recv))
"#;

// ── Inheritance queries ─────────────────────────────────────────────────────
// `@cls` = the subclass/implementing type, `@super` = a base class/trait.
// One match per (subclass, base) pair, so multiple inheritance is captured.

/// Rust: `impl Trait for Type` → Type implements Trait.
pub const RUST_INHERITS: &str = r#"
(impl_item trait: (type_identifier) @super type: (type_identifier) @cls)
"#;

/// Python: `class Sub(Base1, Base2):`.
pub const PYTHON_INHERITS: &str = r#"
(class_definition
  name: (identifier) @cls
  superclasses: (argument_list (identifier) @super))
"#;

/// TypeScript / JavaScript: `class Sub extends Base`.
pub const TS_JS_INHERITS: &str = r#"
(class_declaration
  name: (_) @cls
  (class_heritage (extends_clause (identifier) @super)))
"#;
