//! `pensieve scrape <path>` (one-shot recursive) + `pensieve watch <path>` (foreground)
//! — local-filesystem ingest. Both walk the filesystem and POST each text file
//! to `/v1/agent/files/contribute`, which parses it into the candidate file
//! graph (E5); the dreaming promoter (E6) then stitches files belonging to a
//! known repo to their live upstream nodes.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use crate::client::{effective_config, post_json, ClientConfig};

#[derive(Debug, clap::Args)]
pub(crate) struct ScrapeArgs {
    /// File or directory to scrape (recursive). Respects .gitignore.
    pub path: String,
    /// Glob(s) to include (repeatable). Default: everything not gitignored.
    #[arg(long)]
    pub include: Vec<String>,
    /// Glob(s) to exclude (repeatable).
    #[arg(long)]
    pub exclude: Vec<String>,
    /// Project realm the files belong to.
    #[arg(long, default_value = "default")]
    pub realm: String,
    /// "owner/name" if these files belong to a known repo — enables E6 stitching
    /// to the live upstream code graph.
    #[arg(long)]
    pub repo: Option<String>,
    /// Skip files larger than this many bytes.
    #[arg(long, default_value_t = 1_048_576)]
    pub max_bytes: u64,
}

/// Build the include/exclude glob matcher (shared by `scrape`'s walker and
/// `watch`'s per-event filter). Empty when no globs are given (accept all).
fn build_overrides(root: &Path, args: &ScrapeArgs) -> Result<ignore::overrides::Override> {
    let mut ob = ignore::overrides::OverrideBuilder::new(root);
    for g in &args.include {
        ob.add(g).with_context(|| format!("bad --include glob {g:?}"))?;
    }
    for g in &args.exclude {
        ob.add(&format!("!{g}")).with_context(|| format!("bad --exclude glob {g:?}"))?;
    }
    Ok(ob.build()?)
}

fn build_walker(root: &Path, args: &ScrapeArgs) -> Result<ignore::Walk> {
    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(true).git_ignore(true).git_global(true).git_exclude(true);
    if !args.include.is_empty() || !args.exclude.is_empty() {
        builder.overrides(build_overrides(root, args)?);
    }
    Ok(builder.build())
}

/// Common noise that `watch` should never contribute (it doesn't get the
/// walker's .gitignore handling, so skip the usual build/VCS dirs by path).
fn is_noise(p: &Path) -> bool {
    p.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some(".git") | Some("target") | Some("node_modules") | Some("dist") | Some(".venv")
        )
    })
}

/// Read one file (if text + under the size cap) and contribute it. Returns the
/// symbol count on success, `None` when skipped/failed.
async fn contribute_one(cfg: &ClientConfig, root: &Path, p: &Path, args: &ScrapeArgs) -> Option<u64> {
    let meta = p.metadata().ok()?;
    if !meta.is_file() || meta.len() > args.max_bytes {
        return None;
    }
    // Skip binaries: only valid-UTF-8 content is contributed.
    let content = String::from_utf8(std::fs::read(p).ok()?).ok()?;
    let rel = p.strip_prefix(root).unwrap_or(p).to_string_lossy().to_string();
    let body = json!({
        "path": rel,
        "realm": args.realm,
        "repo": args.repo,
        "content": content,
        "why_read": format!("pensieve scrape: {}", p.display()),
    });
    match post_json(cfg, "/v1/agent/files/contribute", body).await {
        Ok(v) => Some(v.get("symbols").and_then(|s| s.as_u64()).unwrap_or(0)),
        Err(e) => {
            eprintln!("  ! {rel}: {e}");
            None
        }
    }
}

pub(crate) async fn scrape(args: ScrapeArgs) -> Result<()> {
    let cfg = effective_config()?;
    let root = std::fs::canonicalize(&args.path)
        .with_context(|| format!("resolve {}", args.path))?;

    let mut sent = 0usize;
    let mut symbols = 0u64;
    for result in build_walker(&root, &args)? {
        let Ok(entry) = result else { continue };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        if let Some(n) = contribute_one(&cfg, &root, entry.path(), &args).await {
            sent += 1;
            symbols += n;
            if sent % 50 == 0 {
                println!("  … {sent} files");
            }
        }
    }
    println!(
        "scraped {sent} file(s) from {} → {symbols} symbol(s) into the file_candidates graph",
        root.display()
    );
    Ok(())
}

pub(crate) async fn watch(args: ScrapeArgs) -> Result<()> {
    use notify::{EventKind, RecursiveMode, Watcher};

    let cfg = effective_config()?;
    let root = std::fs::canonicalize(&args.path)
        .with_context(|| format!("resolve {}", args.path))?;

    // notify's callback is sync; forward events to an async channel.
    // Apply the same --include/--exclude globs scrape uses, plus a noise-dir
    // skip (watch has no walker to honour .gitignore).
    let overrides = build_overrides(&root, &args)?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .context("create filesystem watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", root.display()))?;

    println!("watching {} for changes … (Ctrl-C to stop)", root.display());
    while let Some(res) = rx.recv().await {
        let Ok(event) = res else { continue };
        if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
            continue;
        }
        for p in event.paths {
            if p.is_file() && !is_noise(&p) && !overrides.matched(&p, false).is_ignore() {
                if let Some(n) = contribute_one(&cfg, &root, &p, &args).await {
                    let rel = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().to_string();
                    println!("  ✓ {rel} ({n} symbols)");
                }
            }
        }
    }
    Ok(())
}
