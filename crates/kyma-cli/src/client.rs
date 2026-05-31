//! Client-side helpers: persisted config + agent SSE streaming.

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct ClientConfig {
    pub(crate) endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) token: Option<String>,
    /// Last conversation session id, set after each `query`. `--continue`
    /// resumes this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_session_id: Option<String>,
}

pub(crate) fn config_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".kyma"));
    }
    Err(anyhow!("$HOME is not set"))
}

pub(crate) fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

pub(crate) fn load_config() -> Result<ClientConfig> {
    let p = config_path()?;
    let raw = std::fs::read_to_string(&p)
        .with_context(|| format!("read {}", p.display()))?;
    let cfg: ClientConfig = serde_json::from_str(&raw)
        .with_context(|| format!("parse {}", p.display()))?;
    Ok(cfg)
}

pub(crate) fn save_config(cfg: &ClientConfig) -> Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let path = config_path()?;
    let json = serde_json::to_string_pretty(cfg)?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Resolve the effective config: precedence is `KYMA_SERVER_URL` /
/// `KYMA_TOKEN` env vars, then `~/.kyma/config.json`. Env wins so the
/// CLI works in CI without needing the file.
pub(crate) fn effective_config() -> Result<ClientConfig> {
    let mut cfg = load_config().unwrap_or_default();
    if let Ok(v) = std::env::var("KYMA_SERVER_URL") {
        if !v.is_empty() {
            cfg.endpoint = v;
        }
    }
    if let Ok(v) = std::env::var("KYMA_TOKEN") {
        if !v.is_empty() {
            cfg.token = Some(v);
        }
    }
    if cfg.endpoint.is_empty() {
        return Err(anyhow!(
            "no server configured — run `kyma connect <url>` or set KYMA_SERVER_URL"
        ));
    }
    Ok(cfg)
}

pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("kyma-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("reqwest client")
}

/// Stream `/v1/agent/ask` SSE. For each event we emit (event_name, json_data).
/// The caller renders.
pub(crate) async fn stream_agent_ask(
    cfg: &ClientConfig,
    question: &str,
    session_id: Option<&str>,
    mut on_event: impl FnMut(&str, &str),
) -> Result<()> {
    let url = format!("{}/v1/agent/ask", cfg.endpoint.trim_end_matches('/'));
    let mut body = serde_json::json!({
        "question": question,
        "include_thinking": false,
    });
    if let Some(sid) = session_id {
        body["session_id"] = serde_json::Value::String(sid.to_string());
    }
    let mut req = http_client().post(url).json(&body);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    req = req.header("accept", "text/event-stream");
    let res = req.send().await.context("posting /v1/agent/ask")?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(anyhow!("server returned {status}: {body}"));
    }

    let mut stream = res.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("reading SSE chunk")?;
        buffer.extend_from_slice(&bytes);
        while let Some(pos) = find_double_newline(&buffer) {
            let frame = buffer.drain(..pos + 2).collect::<Vec<u8>>();
            let s = String::from_utf8_lossy(&frame);
            let (event, data) = parse_sse_frame(&s);
            on_event(&event, &data);
        }
    }
    Ok(())
}

fn find_double_newline(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

fn parse_sse_frame(frame: &str) -> (String, String) {
    let mut event = String::new();
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim());
        }
    }
    (event, data)
}

/// GET a JSON endpoint with the configured bearer token.
pub(crate) async fn get_json(cfg: &ClientConfig, path: &str) -> Result<serde_json::Value> {
    let url = format!("{}{}", cfg.endpoint.trim_end_matches('/'), path);
    let mut req = http_client().get(url);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.with_context(|| format!("GET {path}"))?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("server returned {status}: {text}"));
    }
    Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
}

/// DELETE a JSON endpoint with the configured bearer token.
pub(crate) async fn delete_json(cfg: &ClientConfig, path: &str) -> Result<serde_json::Value> {
    let url = format!("{}{}", cfg.endpoint.trim_end_matches('/'), path);
    let mut req = http_client().delete(url);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.with_context(|| format!("DELETE {path}"))?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("server returned {status}: {text}"));
    }
    Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
}

pub(crate) async fn probe_health(cfg: &ClientConfig) -> Result<String> {
    let url = format!("{}/health", cfg.endpoint.trim_end_matches('/'));
    let res = http_client().get(url).send().await.context("GET /health")?;
    if !res.status().is_success() {
        return Err(anyhow!("health returned {}", res.status()));
    }
    Ok(res.text().await.unwrap_or_default())
}

/// Resolve the install target dir. If `explicit` is given, use it; else
/// default to `$HOME/.kyma/skills/kyma/`. Creates the dir.
pub(crate) fn install_target(explicit: Option<PathBuf>) -> Result<PathBuf> {
    let dir = match explicit {
        Some(p) => p,
        None => config_dir()?.join("skills").join("kyma"),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    Ok(dir)
}

pub(crate) fn write_skill_file(target: &Path, body: &str) -> Result<PathBuf> {
    let path = target.join("SKILL.md");
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}
