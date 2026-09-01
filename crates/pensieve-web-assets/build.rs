use std::path::Path;
use std::{env, fs};

/// `lib.rs` embeds the web UI from the crate-local `embedded/` directory via
/// `include_dir!`. Keeping the assets *inside* the crate is what makes it
/// publishable to crates.io: `../../web/dist` escapes the package root, so a
/// published `.crate` could never carry it.
///
/// Resolution order:
///   1. `../../web/dist` exists (workspace + CI builds, after `pnpm -C web
///      build`) → mirror it into `embedded/`.
///   2. Otherwise `embedded/index.html` already exists (a published crate ships
///      `embedded/` via the `include` list, and the source tree is absent) →
///      keep it.
///   3. Neither → seed a minimal placeholder so the macro and a plain
///      `cargo build` on a fresh checkout still succeed.
fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let embedded = Path::new(&manifest).join("embedded");
    let src = Path::new(&manifest).join("../../web/dist");

    if src.join("index.html").exists() {
        // Workspace / CI build: mirror the freshly-built UI into the crate.
        let _ = fs::remove_dir_all(&embedded);
        copy_dir(&src, &embedded).expect("copying web/dist -> embedded");
    } else if !embedded.join("index.html").exists() {
        // No source tree and nothing bundled: seed a placeholder.
        fs::create_dir_all(embedded.join("assets")).ok();
        let _ = fs::write(
            embedded.join("index.html"),
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
             <title>pensieve</title></head><body style=\"font-family:sans-serif;padding:2rem\">\
             <h1>pensieve</h1><p>The web UI was not built. Run <code>pnpm -C web build</code> \
             and rebuild to embed the interface.</p></body></html>",
        );
    }
    // else: published crate ships `embedded/` — keep it as-is.

    println!("cargo:rerun-if-changed=../../web/dist");
}

/// Recursively copy `from` into `to` using only std (no build-deps).
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dst)?;
        } else {
            fs::copy(entry.path(), &dst)?;
        }
    }
    Ok(())
}
