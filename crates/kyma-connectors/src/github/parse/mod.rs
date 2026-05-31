//! Tree-sitter–based structural code parser.
//!
//! ## Scope (P1 — structural only)
//! Extracts **import** statements from source files.  Function/class nodes
//! and call-graph edges are deferred to P2 (`code_intel`).
//!
//! ## Supported languages
//! | Extension(s)     | Grammar              |
//! |------------------|----------------------|
//! | `.rs`            | tree-sitter-rust     |
//! | `.py`            | tree-sitter-python   |
//! | `.ts` / `.tsx`   | tree-sitter-typescript (typescript) |
//! | `.js` / `.jsx`   | tree-sitter-javascript |
//! | `.go`            | tree-sitter-go       |
//!
//! Unknown extensions are silently skipped.

pub mod queries;

use tree_sitter::{Language, Node, Parser, Query, QueryCursor};
use tree_sitter::StreamingIterator as _;

/// Supported languages — maps to the grammar crates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
}

impl Lang {
    /// Infer the language from a file path extension.
    pub fn from_path(path: &str) -> Option<Self> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())?;
        match ext {
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            "ts" | "tsx" => Some(Self::TypeScript),
            "js" | "jsx" => Some(Self::JavaScript),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    /// Return the tree-sitter `Language` for this variant.
    pub fn ts_language(self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::TypeScript => {
                // tree-sitter-typescript exposes two grammars: `typescript`
                // and `tsx`.  We use `typescript` for both `.ts` and `.tsx`
                // here because P1 only needs import paths, not JSX semantics.
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            }
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    /// The query string to use for import extraction.
    fn query_str(self) -> &'static str {
        match self {
            Self::Rust => queries::RUST_IMPORTS,
            Self::Python => queries::PYTHON_IMPORTS,
            Self::TypeScript | Self::JavaScript => queries::TS_JS_IMPORTS,
            Self::Go => queries::GO_IMPORTS,
        }
    }

    /// The query string for definition (function/class) extraction.
    fn defs_query_str(self) -> &'static str {
        match self {
            Self::Rust => queries::RUST_DEFS,
            Self::Python => queries::PYTHON_DEFS,
            Self::TypeScript | Self::JavaScript => queries::TS_JS_DEFS,
            Self::Go => queries::GO_DEFS,
        }
    }

    /// The query string for call-site extraction.
    fn calls_query_str(self) -> &'static str {
        match self {
            Self::Rust => queries::RUST_CALLS,
            Self::Python => queries::PYTHON_CALLS,
            Self::TypeScript | Self::JavaScript => queries::TS_JS_CALLS,
            Self::Go => queries::GO_CALLS,
        }
    }

    /// The query string for inheritance extraction (`None` if unsupported).
    fn inherits_query_str(self) -> Option<&'static str> {
        match self {
            Self::Rust => Some(queries::RUST_INHERITS),
            Self::Python => Some(queries::PYTHON_INHERITS),
            Self::TypeScript | Self::JavaScript => Some(queries::TS_JS_INHERITS),
            Self::Go => None,
        }
    }

    /// Tree-sitter node kinds that introduce a named function scope.
    fn fn_def_kinds(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["function_item"],
            Self::Python => &["function_definition"],
            Self::TypeScript | Self::JavaScript => &["function_declaration", "method_definition"],
            Self::Go => &["function_declaration", "method_declaration"],
        }
    }

    /// Name shown in node `props`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Go => "go",
        }
    }
}

/// A raw import target extracted from a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImport {
    /// The raw import target string (e.g. `"crate::foo"`, `"./bar"`, `"fmt"`).
    pub raw: String,
    /// 1-based line number of the import statement.
    pub line: usize,
}

/// Parse `source` using `lang` and return all import targets found.
///
/// This function is synchronous and CPU-bound; callers in async context should
/// wrap it with `tokio::task::spawn_blocking`.
///
/// Returns an empty `Vec` (not an error) if the source cannot be parsed or
/// the query produces no matches — best-effort extraction.
pub fn extract_imports(lang: Lang, source: &str) -> Vec<RawImport> {
    let ts_lang = lang.ts_language();
    let query_str = lang.query_str();

    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return vec![];
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };

    let query = match Query::new(&ts_lang, query_str) {
        Ok(q) => q,
        Err(_) => return vec![],
    };

    let import_capture_idx = match query.capture_index_for_name("import") {
        Some(idx) => idx,
        None => return vec![],
    };

    let mut qcursor = QueryCursor::new();
    let source_bytes = source.as_bytes();
    let mut matches = qcursor.matches(&query, tree.root_node(), source_bytes);

    let mut imports = Vec::new();
    while let Some(m) = matches.next() {
        for cap in m.captures {
            if cap.index != import_capture_idx {
                continue;
            }
            let node = cap.node;
            let raw_text = node.utf8_text(source_bytes).unwrap_or("").to_string();
            // Strip surrounding quotes from string literals (TS/JS/Go).
            let raw = strip_quotes(&raw_text);
            let line = node.start_position().row + 1; // 0-indexed → 1-indexed
            imports.push(RawImport { raw, line });
        }
    }

    // Deduplicate by (raw, line) — a single import line might match multiple
    // sub-queries; keep first occurrence per raw string + line.
    imports.dedup_by(|a, b| a.raw == b.raw && a.line == b.line);
    imports
}

/// The kind of a code definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Class,
}

/// A function/method or class/type definition extracted from a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSymbol {
    pub kind: SymbolKind,
    /// The defined symbol's name (e.g. `parse_config`, `HttpClient`).
    pub name: String,
    /// 1-based line of the definition.
    pub line: usize,
}

/// Parse `source` and return the function/method and class/type definitions.
///
/// Synchronous + CPU-bound — wrap in `spawn_blocking` from async code.
/// Best-effort: returns an empty `Vec` if the source can't be parsed.
pub fn extract_symbols(lang: Lang, source: &str) -> Vec<CodeSymbol> {
    let ts_lang = lang.ts_language();
    let query_str = lang.defs_query_str();

    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return vec![];
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };
    let query = match Query::new(&ts_lang, query_str) {
        Ok(q) => q,
        Err(_) => return vec![],
    };
    let func_idx = query.capture_index_for_name("func");
    let class_idx = query.capture_index_for_name("class");

    let mut qcursor = QueryCursor::new();
    let source_bytes = source.as_bytes();
    let mut matches = qcursor.matches(&query, tree.root_node(), source_bytes);

    let mut out = Vec::new();
    while let Some(m) = matches.next() {
        for cap in m.captures {
            let kind = if Some(cap.index) == func_idx {
                SymbolKind::Function
            } else if Some(cap.index) == class_idx {
                SymbolKind::Class
            } else {
                continue;
            };
            let name = cap.node.utf8_text(source_bytes).unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let line = cap.node.start_position().row + 1;
            out.push(CodeSymbol { kind, name, line });
        }
    }
    out.dedup_by(|a, b| a.kind == b.kind && a.name == b.name && a.line == b.line);
    out
}

/// A call site: who calls what, and whether it's via a receiver (`obj.m()`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCall {
    /// Enclosing function name, if the call is inside one.
    pub caller: Option<String>,
    /// Called name: the function (`foo()`) or the receiver root (`boto3.x()`).
    pub callee: String,
    /// 1-based line of the call.
    pub line: usize,
    /// True when `callee` is a receiver root (`boto3` in `boto3.client()`),
    /// used to link functions to the libraries/modules they use.
    pub via_receiver: bool,
}

/// Walk up from `node` to the nearest enclosing function definition and return
/// its name.
fn enclosing_function_name(node: Node, lang: Lang, src: &[u8]) -> Option<String> {
    let kinds = lang.fn_def_kinds();
    let mut cur = node.parent();
    while let Some(p) = cur {
        if kinds.contains(&p.kind()) {
            if let Some(name) = p.child_by_field_name("name") {
                return name.utf8_text(src).ok().map(|s| s.to_string());
            }
        }
        cur = p.parent();
    }
    None
}

/// Parse `source` and return the call sites (direct calls + receiver roots).
///
/// Synchronous + CPU-bound — wrap in `spawn_blocking` from async code.
pub fn extract_calls(lang: Lang, source: &str) -> Vec<RawCall> {
    let ts_lang = lang.ts_language();
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return vec![];
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };
    let query = match Query::new(&ts_lang, lang.calls_query_str()) {
        Ok(q) => q,
        Err(_) => return vec![],
    };
    let fn_idx = query.capture_index_for_name("call_fn");
    let recv_idx = query.capture_index_for_name("call_recv");

    let mut qcursor = QueryCursor::new();
    let src = source.as_bytes();
    let mut matches = qcursor.matches(&query, tree.root_node(), src);

    let mut out = Vec::new();
    while let Some(m) = matches.next() {
        for cap in m.captures {
            let via_receiver = Some(cap.index) == recv_idx;
            if Some(cap.index) != fn_idx && !via_receiver {
                continue;
            }
            let callee = cap.node.utf8_text(src).unwrap_or("").to_string();
            if callee.is_empty() {
                continue;
            }
            let caller = enclosing_function_name(cap.node, lang, src);
            let line = cap.node.start_position().row + 1;
            out.push(RawCall { caller, callee, line, via_receiver });
        }
    }
    out
}

/// A class-inheritance relationship (subclass → base class/trait).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawInherit {
    pub subclass: String,
    pub superclass: String,
    /// 1-based line of the subclass definition.
    pub line: usize,
}

/// Parse `source` and return inheritance pairs (`class Sub(Base)`,
/// `impl Trait for Type`, `class Sub extends Base`). Empty for Go.
pub fn extract_inherits(lang: Lang, source: &str) -> Vec<RawInherit> {
    let Some(query_str) = lang.inherits_query_str() else { return vec![] };
    let ts_lang = lang.ts_language();
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return vec![];
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![],
    };
    let query = match Query::new(&ts_lang, query_str) {
        Ok(q) => q,
        Err(_) => return vec![],
    };
    let cls_idx = query.capture_index_for_name("cls");
    let super_idx = query.capture_index_for_name("super");

    let mut qcursor = QueryCursor::new();
    let src = source.as_bytes();
    let mut matches = qcursor.matches(&query, tree.root_node(), src);

    let mut out = Vec::new();
    while let Some(m) = matches.next() {
        let mut subclass = None;
        let mut superclass = None;
        let mut line = 0usize;
        for cap in m.captures {
            if Some(cap.index) == cls_idx {
                subclass = cap.node.utf8_text(src).ok().map(|s| s.to_string());
                line = cap.node.start_position().row + 1;
            } else if Some(cap.index) == super_idx {
                superclass = cap.node.utf8_text(src).ok().map(|s| s.to_string());
            }
        }
        if let (Some(c), Some(s)) = (subclass, superclass) {
            if !c.is_empty() && !s.is_empty() && c != s {
                out.push(RawInherit { subclass: c, superclass: s, line });
            }
        }
    }
    out
}

/// Strip surrounding `"`, `'`, or `` ` `` from a string literal.
fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('\'') && s.ends_with('\''))
        || (s.starts_with('`') && s.ends_with('`'))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Rust ──────────────────────────────────────────────────────────────────

    #[test]
    fn rust_use_crate() {
        let src = r#"
use crate::foo::bar;
use std::collections::HashMap;
use super::baz;
"#;
        let imports = extract_imports(Lang::Rust, src);
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert!(raws.iter().any(|r| r.contains("crate") || r.contains("foo")), "expected crate import, got: {:?}", raws);
        assert!(raws.iter().any(|r| r.contains("std") || r.contains("collections")), "expected std import, got: {:?}", raws);
    }

    #[test]
    fn rust_mod_item() {
        let src = r#"
mod helpers;
mod internal {
    use super::*;
}
"#;
        let imports = extract_imports(Lang::Rust, src);
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert!(raws.contains(&"helpers"), "expected 'helpers' mod, got: {:?}", raws);
        assert!(raws.contains(&"internal"), "expected 'internal' mod, got: {:?}", raws);
    }

    #[test]
    fn rust_line_numbers() {
        let src = "use std::io;\nuse std::fs;\n";
        let imports = extract_imports(Lang::Rust, src);
        // Line numbers should be 1 and 2
        let lines: Vec<usize> = imports.iter().map(|i| i.line).collect();
        assert!(lines.contains(&1) || lines.contains(&2), "expected lines 1 or 2, got: {:?}", lines);
    }

    // ── Python ────────────────────────────────────────────────────────────────

    #[test]
    fn python_import_statement() {
        let src = "import os\nimport sys\nfrom pathlib import Path\n";
        let imports = extract_imports(Lang::Python, src);
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert!(raws.contains(&"os"), "expected 'os', got: {:?}", raws);
        assert!(raws.contains(&"sys"), "expected 'sys', got: {:?}", raws);
        assert!(raws.contains(&"pathlib"), "expected 'pathlib', got: {:?}", raws);
    }

    #[test]
    fn python_from_import() {
        let src = "from .relative import foo\nfrom ..parent import bar\n";
        let imports = extract_imports(Lang::Python, src);
        // relative imports come back as relative_import node text
        assert!(!imports.is_empty(), "expected relative imports, got empty");
    }

    #[test]
    fn python_line_number() {
        let src = "# comment\nimport json\n";
        let imports = extract_imports(Lang::Python, src);
        let lines: Vec<usize> = imports.iter().map(|i| i.line).collect();
        // "import json" is on line 2
        assert!(lines.contains(&2), "expected line 2, got: {:?}", lines);
    }

    // ── TypeScript ────────────────────────────────────────────────────────────

    #[test]
    fn typescript_es_import() {
        let src = r#"
import { foo } from "./foo";
import * as bar from "../bar";
import type { Baz } from "@scope/pkg";
"#;
        let imports = extract_imports(Lang::TypeScript, src);
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert!(raws.contains(&"./foo"), "expected './foo', got: {:?}", raws);
        assert!(raws.contains(&"../bar"), "expected '../bar', got: {:?}", raws);
        assert!(raws.contains(&"@scope/pkg"), "expected '@scope/pkg', got: {:?}", raws);
    }

    #[test]
    fn typescript_require() {
        let src = r#"const x = require("express");"#;
        let imports = extract_imports(Lang::TypeScript, src);
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert!(raws.contains(&"express"), "expected 'express', got: {:?}", raws);
    }

    // ── JavaScript ────────────────────────────────────────────────────────────

    #[test]
    fn javascript_es_import() {
        let src = r#"import React from "react";"#;
        let imports = extract_imports(Lang::JavaScript, src);
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert!(raws.contains(&"react"), "expected 'react', got: {:?}", raws);
    }

    #[test]
    fn javascript_require() {
        let src = r#"const fs = require("fs");"#;
        let imports = extract_imports(Lang::JavaScript, src);
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert!(raws.contains(&"fs"), "expected 'fs', got: {:?}", raws);
    }

    // ── Go ────────────────────────────────────────────────────────────────────

    #[test]
    fn go_single_import() {
        let src = r#"package main
import "fmt"
"#;
        let imports = extract_imports(Lang::Go, src);
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert!(raws.contains(&"fmt"), "expected 'fmt', got: {:?}", raws);
    }

    #[test]
    fn go_grouped_import() {
        let src = r#"package main
import (
    "fmt"
    "os"
    "github.com/foo/bar"
)
"#;
        let imports = extract_imports(Lang::Go, src);
        let raws: Vec<&str> = imports.iter().map(|i| i.raw.as_str()).collect();
        assert!(raws.contains(&"fmt"), "expected 'fmt', got: {:?}", raws);
        assert!(raws.contains(&"os"), "expected 'os', got: {:?}", raws);
        assert!(raws.contains(&"github.com/foo/bar"), "expected 'github.com/foo/bar', got: {:?}", raws);
    }

    // ── lang_from_path ────────────────────────────────────────────────────────

    #[test]
    fn lang_from_path_known_extensions() {
        assert_eq!(Lang::from_path("src/main.rs"), Some(Lang::Rust));
        assert_eq!(Lang::from_path("app.py"), Some(Lang::Python));
        assert_eq!(Lang::from_path("index.ts"), Some(Lang::TypeScript));
        assert_eq!(Lang::from_path("comp.tsx"), Some(Lang::TypeScript));
        assert_eq!(Lang::from_path("util.js"), Some(Lang::JavaScript));
        assert_eq!(Lang::from_path("comp.jsx"), Some(Lang::JavaScript));
        assert_eq!(Lang::from_path("main.go"), Some(Lang::Go));
    }

    #[test]
    fn lang_from_path_unknown_extension() {
        assert_eq!(Lang::from_path("README.md"), None);
        assert_eq!(Lang::from_path("config.toml"), None);
        assert_eq!(Lang::from_path("noext"), None);
    }

    // ── empty / invalid source ────────────────────────────────────────────────

    #[test]
    fn empty_source_returns_empty() {
        assert_eq!(extract_imports(Lang::Rust, ""), vec![]);
    }

    #[test]
    fn no_imports_returns_empty() {
        let src = "fn main() { println!(\"hello\"); }\n";
        let imports = extract_imports(Lang::Rust, src);
        assert!(imports.is_empty(), "expected no imports, got: {:?}", imports);
    }

    // ── symbol extraction (functions + classes) ─────────────────────────────────

    fn names(syms: &[CodeSymbol], kind: SymbolKind) -> Vec<&str> {
        syms.iter().filter(|s| s.kind == kind).map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn python_symbols_functions_and_classes() {
        let src = "import os\n\nclass Client:\n    def connect(self):\n        pass\n\ndef main():\n    pass\n";
        let syms = extract_symbols(Lang::Python, src);
        assert!(names(&syms, SymbolKind::Class).contains(&"Client"), "classes: {:?}", syms);
        let fns = names(&syms, SymbolKind::Function);
        assert!(fns.contains(&"connect"), "fns: {:?}", fns);
        assert!(fns.contains(&"main"), "fns: {:?}", fns);
        // line numbers are 1-based
        assert!(syms.iter().any(|s| s.name == "main" && s.line == 7), "main line: {:?}", syms);
    }

    #[test]
    fn rust_symbols_functions_structs_traits() {
        let src = "struct Server;\ntrait Handler { fn handle(&self); }\nfn run() {}\nenum State { On, Off }\n";
        let syms = extract_symbols(Lang::Rust, src);
        let classes = names(&syms, SymbolKind::Class);
        assert!(classes.contains(&"Server"), "classes: {:?}", classes);
        assert!(classes.contains(&"Handler"), "classes: {:?}", classes);
        assert!(classes.contains(&"State"), "classes: {:?}", classes);
        let fns = names(&syms, SymbolKind::Function);
        assert!(fns.contains(&"run"), "fns: {:?}", fns);
        assert!(fns.contains(&"handle"), "fns: {:?}", fns);
    }

    #[test]
    fn ts_symbols_class_and_methods() {
        let src = "export class Repo {\n  fetch() {}\n}\nfunction helper() {}\n";
        let syms = extract_symbols(Lang::TypeScript, src);
        assert!(names(&syms, SymbolKind::Class).contains(&"Repo"), "classes: {:?}", syms);
        let fns = names(&syms, SymbolKind::Function);
        assert!(fns.contains(&"helper"), "fns: {:?}", fns);
        assert!(fns.contains(&"fetch"), "fns: {:?}", fns);
    }

    #[test]
    fn go_symbols_funcs_methods_types() {
        let src = "package main\ntype Server struct {}\nfunc (s *Server) Start() {}\nfunc main() {}\n";
        let syms = extract_symbols(Lang::Go, src);
        assert!(names(&syms, SymbolKind::Class).contains(&"Server"), "classes: {:?}", syms);
        let fns = names(&syms, SymbolKind::Function);
        assert!(fns.contains(&"Start"), "fns: {:?}", fns);
        assert!(fns.contains(&"main"), "fns: {:?}", fns);
    }

    #[test]
    fn symbols_empty_source() {
        assert_eq!(extract_symbols(Lang::Python, ""), vec![]);
    }

    // ── call extraction ─────────────────────────────────────────────────────────

    #[test]
    fn python_calls_direct_and_receiver() {
        let src = "import boto3\n\ndef main():\n    helper()\n    boto3.client('iam')\n\ndef helper():\n    pass\n";
        let calls = extract_calls(Lang::Python, src);
        // direct call helper() inside main → caller=main, callee=helper
        assert!(
            calls.iter().any(|c| c.caller.as_deref() == Some("main") && c.callee == "helper" && !c.via_receiver),
            "expected main→helper, got: {:?}", calls
        );
        // receiver boto3 inside main → caller=main, callee=boto3, via_receiver
        assert!(
            calls.iter().any(|c| c.caller.as_deref() == Some("main") && c.callee == "boto3" && c.via_receiver),
            "expected main→boto3 (receiver), got: {:?}", calls
        );
    }

    #[test]
    fn rust_calls_caller_resolution() {
        let src = "fn run() {\n    helper();\n    std::fs::read(\"x\");\n}\nfn helper() {}\n";
        let calls = extract_calls(Lang::Rust, src);
        assert!(
            calls.iter().any(|c| c.caller.as_deref() == Some("run") && c.callee == "helper"),
            "expected run→helper, got: {:?}", calls
        );
    }

    #[test]
    fn calls_empty_source() {
        assert_eq!(extract_calls(Lang::Python, ""), vec![]);
    }

    // ── inheritance ─────────────────────────────────────────────────────────────

    #[test]
    fn python_inherits_base_class() {
        let src = "class Base:\n    pass\n\nclass Sub(Base):\n    pass\n";
        let inh = extract_inherits(Lang::Python, src);
        assert!(
            inh.iter().any(|i| i.subclass == "Sub" && i.superclass == "Base"),
            "expected Sub→Base, got: {:?}", inh
        );
    }

    #[test]
    fn rust_inherits_impl_trait() {
        let src = "trait Greet {}\nstruct Robot;\nimpl Greet for Robot {}\n";
        let inh = extract_inherits(Lang::Rust, src);
        assert!(
            inh.iter().any(|i| i.subclass == "Robot" && i.superclass == "Greet"),
            "expected Robot→Greet, got: {:?}", inh
        );
    }

    #[test]
    fn ts_inherits_extends() {
        let src = "class Base {}\nclass Sub extends Base {}\n";
        let inh = extract_inherits(Lang::TypeScript, src);
        assert!(
            inh.iter().any(|i| i.subclass == "Sub" && i.superclass == "Base"),
            "expected Sub→Base, got: {:?}", inh
        );
    }

    #[test]
    fn go_inherits_empty() {
        assert_eq!(extract_inherits(Lang::Go, "package main\ntype X struct{}\n"), vec![]);
    }
}
