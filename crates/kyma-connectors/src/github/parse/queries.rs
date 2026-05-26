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
