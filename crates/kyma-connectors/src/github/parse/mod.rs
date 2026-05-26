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

use tree_sitter::{Language, Parser, Query, QueryCursor};
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
}
