//! Embedded Web UI assets — compiled into the binary at build time.
//!
//! The caller (kyma-server) iterates these behind the `web-ui` feature.
#![forbid(unsafe_code)]

use include_dir::{include_dir, Dir};

/// The Web UI's `dist/` directory, baked in at compile time.
pub static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../web/dist");

/// MIME type for a filename, by extension. Falls back to `application/octet-stream`.
pub fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg"  => "image/svg+xml",
        "png"  => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ico"  => "image/x-icon",
        "map"  => "application/json",
        _ => "application/octet-stream",
    }
}
