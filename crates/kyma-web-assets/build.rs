use std::path::Path;

/// `lib.rs` embeds `../../web/dist` via `include_dir!`, which fails to compile
/// if that directory doesn't exist. The real UI is produced by `pnpm -C web
/// build` (run by the Docker build and `kyma-local`'s build step). To keep a
/// plain `cargo build` working on a fresh checkout — where `web/dist` is
/// gitignored and may be absent — seed a minimal placeholder when it's missing.
/// A real build overwrites it.
fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dist = Path::new(&manifest).join("../../web/dist");
    let index = dist.join("index.html");
    if !index.exists() {
        std::fs::create_dir_all(dist.join("assets")).ok();
        let _ = std::fs::write(
            &index,
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
             <title>kyma</title></head><body style=\"font-family:sans-serif;padding:2rem\">\
             <h1>kyma</h1><p>The web UI was not built. Run <code>pnpm -C web build</code> \
             and rebuild to embed the interface.</p></body></html>",
        );
    }
    println!("cargo:rerun-if-changed=../../web/dist");
}
