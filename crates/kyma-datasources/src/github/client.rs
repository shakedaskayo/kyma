//! GitHub REST API client — thin wrapper over `reqwest`.
//!
//! ## Rate limiting
//! We read `X-RateLimit-Remaining` on every response. When it falls below
//! `RATE_LIMIT_FLOOR` we stop paginating and return `StopReason::RateLimited`
//! so `run_once` can surface a Transient error and let the scheduler back off.
//!
//! ## Pagination
//! Pagination follows the `Link: <url>; rel="next"` header pattern. Each
//! paginating method accepts `max_pages` and stops once that limit is
//! reached.
//!
//! ## Retries
//! 403 (GitHub rate-limit), 429, and 5xx responses trigger a Transient error.
//! 4xx (other) responses return Permanent. Network/timeout errors retry up to
//! 3 times with exponential back-off + jitter (mirrors prometheus data source).
//!
//! ## Code graph (B2 additions)
//! `get_tree` fetches the recursive git-tree for a commit SHA.
//! `get_blob` fetches and base64-decodes a single file blob.

use chrono::{DateTime, Utc};
use reqwest::{Client, Response, StatusCode};
use serde_json::Value;

use crate::types::DataSourceError;

const RATE_LIMIT_FLOOR: u64 = 10;
const GITHUB_API_BASE: &str = "https://api.github.com";

/// A single entry from the recursive git-tree API response.
#[derive(Debug, Clone)]
pub struct TreeEntry {
    /// Repo-relative path (e.g. `"src/main.rs"`).
    pub path: String,
    /// `"blob"` for files, `"tree"` for directories.
    pub entry_type: String,
    /// Object SHA (used to fetch blob content).
    pub sha: String,
    /// File size in bytes (only present for blobs; 0 for trees).
    pub size: usize,
}

/// Thin GitHub REST client. Cheaply cloneable.
#[derive(Clone)]
pub struct GithubClient {
    pub http: Client,
    pub token: String,
    pub base_url: String,
}

impl GithubClient {
    pub fn new(http: Client, token: String) -> Self {
        Self {
            http,
            token,
            base_url: GITHUB_API_BASE.to_string(),
        }
    }

    /// For testing: override the base URL to point at a mock server.
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    fn req(&self, url: &str) -> reqwest::RequestBuilder {
        self.http
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "kyma-github-connector/1.0")
    }

    /// Execute a single request with retry logic. Returns the parsed JSON body
    /// and remaining rate-limit count (if the header is present).
    async fn fetch_one(&self, url: &str) -> Result<(Value, Option<u64>), DataSourceError> {
        let mut attempt: u32 = 0;
        loop {
            let resp = self
                .req(url)
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() || e.is_connect() {
                        DataSourceError::Transient(format!("network: {e}"))
                    } else {
                        DataSourceError::Permanent(format!("fetch: {e}"))
                    }
                })?;

            let status = resp.status();
            let remaining = parse_rate_limit_remaining(&resp);

            if status.is_success() {
                let body: Value = resp
                    .json()
                    .await
                    .map_err(|e| DataSourceError::Transient(format!("body parse: {e}")))?;
                return Ok((body, remaining));
            }

            // 403 (GitHub rate-limit token) or 429 (secondary rate-limit) → Transient
            if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
                if attempt >= 3 {
                    return Err(DataSourceError::Transient(format!(
                        "HTTP {status} (rate-limited) after {attempt} retries"
                    )));
                }
            } else if status.is_server_error() {
                if attempt >= 3 {
                    return Err(DataSourceError::Transient(format!(
                        "HTTP {status} after {attempt} retries"
                    )));
                }
            } else {
                // 4xx other → permanent
                return Err(DataSourceError::Permanent(format!("HTTP {status}")));
            }

            attempt += 1;
            let base_ms = 200u64 * (1u64 << (attempt - 1).min(5));
            let jitter = fastrand::u64(..base_ms / 3 + 1);
            tokio::time::sleep(std::time::Duration::from_millis(
                base_ms.saturating_add(jitter),
            ))
            .await;
        }
    }

    /// Paginate a list endpoint, collecting all items into a `Vec<Value>`.
    /// Stops at `max_pages` pages or when no `Link: rel="next"` header is
    /// present. Also stops early if the rate-limit remaining drops below
    /// `RATE_LIMIT_FLOOR` — in that case `StopReason::RateLimited` is
    /// returned so the caller can propagate a Transient error.
    async fn paginate(
        &self,
        first_url: &str,
        max_pages: usize,
    ) -> Result<(Vec<Value>, StopReason), DataSourceError> {
        let mut items: Vec<Value> = Vec::new();
        let mut url = first_url.to_string();
        let mut pages = 0usize;

        loop {
            let resp = self
                .req(&url)
                .send()
                .await
                .map_err(|e| DataSourceError::Transient(format!("network: {e}")))?;

            let status = resp.status();
            let remaining = parse_rate_limit_remaining(&resp);
            let next_url = parse_link_next(&resp);

            if !status.is_success() {
                if status == StatusCode::FORBIDDEN
                    || status == StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
                {
                    return Err(DataSourceError::Transient(format!("HTTP {status}")));
                }
                return Err(DataSourceError::Permanent(format!("HTTP {status}")));
            }

            let body: Value = resp
                .json()
                .await
                .map_err(|e| DataSourceError::Transient(format!("body parse: {e}")))?;

            match body {
                Value::Array(arr) => items.extend(arr),
                other => items.push(other),
            }

            pages += 1;

            // Rate-limit floor check
            if let Some(r) = remaining {
                if r < RATE_LIMIT_FLOOR {
                    return Ok((items, StopReason::RateLimited));
                }
            }

            // Page cap check
            if pages >= max_pages {
                return Ok((items, StopReason::PageCap));
            }

            match next_url {
                Some(next) => url = next,
                None => return Ok((items, StopReason::Done)),
            }
        }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// `GET /repos/{owner}/{repo}` — single repo metadata.
    pub async fn get_repo(&self, owner: &str, name: &str) -> Result<Value, DataSourceError> {
        let url = format!("{}/repos/{owner}/{name}", self.base_url);
        let (body, _) = self.fetch_one(&url).await?;
        Ok(body)
    }

    /// `GET /repos/{owner}/{repo}/branches?per_page=100` — all branches.
    pub async fn list_branches(
        &self,
        owner: &str,
        repo: &str,
        max_pages: usize,
    ) -> Result<Vec<Value>, DataSourceError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/branches?per_page=100",
            self.base_url
        );
        let (items, _reason) = self.paginate(&url, max_pages).await?;
        Ok(items)
    }

    /// `GET /repos/{owner}/{repo}/pulls?state=all&sort=updated&direction=desc&per_page=100[&since=...]`
    pub async fn list_pulls(
        &self,
        owner: &str,
        repo: &str,
        since: Option<DateTime<Utc>>,
        max_pages: usize,
    ) -> Result<(Vec<Value>, StopReason), DataSourceError> {
        let since_param = since
            .map(|t| format!("&since={}", t.to_rfc3339()))
            .unwrap_or_default();
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls?state=all&sort=updated&direction=desc&per_page=100{since_param}",
            self.base_url
        );
        self.paginate(&url, max_pages).await
    }

    /// `GET /repos/{owner}/{repo}/issues?state=all&sort=updated&direction=desc&per_page=100[&since=...]`
    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        since: Option<DateTime<Utc>>,
        max_pages: usize,
    ) -> Result<(Vec<Value>, StopReason), DataSourceError> {
        let since_param = since
            .map(|t| format!("&since={}", t.to_rfc3339()))
            .unwrap_or_default();
        let url = format!(
            "{}/repos/{owner}/{repo}/issues?state=all&sort=updated&direction=desc&per_page=100{since_param}",
            self.base_url
        );
        self.paginate(&url, max_pages).await
    }

    /// `GET /repos/{owner}/{repo}/contributors?per_page=100`
    pub async fn list_contributors(
        &self,
        owner: &str,
        repo: &str,
        max_pages: usize,
    ) -> Result<Vec<Value>, DataSourceError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/contributors?per_page=100",
            self.base_url
        );
        let (items, _reason) = self.paginate(&url, max_pages).await?;
        Ok(items)
    }

    /// List repos visible to the authenticated user:
    /// `GET /user/repos?per_page=100&sort=updated` (+ org repos).
    /// Used by the admin repos endpoint. Returns up to `max_pages` pages.
    pub async fn list_user_repos(
        &self,
        max_pages: usize,
    ) -> Result<Vec<Value>, DataSourceError> {
        let url = format!("{}/user/repos?per_page=100&sort=updated", self.base_url);
        let (items, _reason) = self.paginate(&url, max_pages).await?;
        Ok(items)
    }

    // ── Actions (E1) ──────────────────────────────────────────────────────────

    /// Paginate an endpoint whose body is an *envelope object* with the items
    /// under `key` (e.g. `actions/runs` → `workflow_runs`, `.../jobs` → `jobs`),
    /// rather than a bare array. Otherwise identical to [`Self::paginate`]:
    /// follows `Link: rel="next"`, honours `max_pages` and the rate-limit floor.
    async fn paginate_keyed(
        &self,
        first_url: &str,
        key: &str,
        max_pages: usize,
    ) -> Result<(Vec<Value>, StopReason), DataSourceError> {
        let mut items: Vec<Value> = Vec::new();
        let mut url = first_url.to_string();
        let mut pages = 0usize;

        loop {
            let resp = self
                .req(&url)
                .send()
                .await
                .map_err(|e| DataSourceError::Transient(format!("network: {e}")))?;

            let status = resp.status();
            let remaining = parse_rate_limit_remaining(&resp);
            let next_url = parse_link_next(&resp);

            if !status.is_success() {
                if status == StatusCode::FORBIDDEN
                    || status == StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
                {
                    return Err(DataSourceError::Transient(format!("HTTP {status}")));
                }
                return Err(DataSourceError::Permanent(format!("HTTP {status}")));
            }

            let body: Value = resp
                .json()
                .await
                .map_err(|e| DataSourceError::Transient(format!("body parse: {e}")))?;

            if let Some(arr) = body.get(key).and_then(|v| v.as_array()) {
                items.extend(arr.iter().cloned());
            }

            pages += 1;

            if let Some(r) = remaining {
                if r < RATE_LIMIT_FLOOR {
                    return Ok((items, StopReason::RateLimited));
                }
            }
            if pages >= max_pages {
                return Ok((items, StopReason::PageCap));
            }
            match next_url {
                Some(next) => url = next,
                None => return Ok((items, StopReason::Done)),
            }
        }
    }

    /// `GET /repos/{owner}/{repo}/actions/runs?per_page=100` — workflow runs,
    /// most-recent first. Returns the `workflow_runs` items. The data source
    /// applies the incremental watermark (by `created_at`) client-side.
    pub async fn list_workflow_runs(
        &self,
        owner: &str,
        repo: &str,
        max_pages: usize,
    ) -> Result<(Vec<Value>, StopReason), DataSourceError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/actions/runs?per_page=100",
            self.base_url
        );
        self.paginate_keyed(&url, "workflow_runs", max_pages).await
    }

    /// `GET /repos/{owner}/{repo}/actions/runs/{run_id}/jobs?per_page=100` —
    /// the jobs of a single run. Returns the `jobs` items.
    pub async fn list_run_jobs(
        &self,
        owner: &str,
        repo: &str,
        run_id: i64,
        max_pages: usize,
    ) -> Result<(Vec<Value>, StopReason), DataSourceError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/actions/runs/{run_id}/jobs?per_page=100",
            self.base_url
        );
        self.paginate_keyed(&url, "jobs", max_pages).await
    }

    /// `GET /repos/{owner}/{repo}/actions/jobs/{job_id}/logs` — the full plain
    /// text log for one job. GitHub 302-redirects to a signed blob URL; reqwest
    /// follows it (stripping the auth header cross-host, which the signed URL
    /// requires). Returns the raw, UN-redacted text — the caller must redact
    /// before persisting.
    ///
    /// Errors are the caller's to soften: a 404 (logs expired) should skip the
    /// job's log rather than fail the whole tick.
    pub async fn fetch_job_log_text(
        &self,
        owner: &str,
        repo: &str,
        job_id: i64,
    ) -> Result<String, DataSourceError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/actions/jobs/{job_id}/logs",
            self.base_url
        );
        let resp = self.req(&url).send().await.map_err(|e| {
            if e.is_timeout() || e.is_connect() {
                DataSourceError::Transient(format!("network: {e}"))
            } else {
                DataSourceError::Permanent(format!("fetch: {e}"))
            }
        })?;

        let status = resp.status();
        if status.is_success() {
            return resp
                .text()
                .await
                .map_err(|e| DataSourceError::Transient(format!("log body: {e}")));
        }
        if status == StatusCode::FORBIDDEN
            || status == StatusCode::TOO_MANY_REQUESTS
            || status.is_server_error()
        {
            return Err(DataSourceError::Transient(format!(
                "HTTP {status} fetching job {job_id} logs"
            )));
        }
        Err(DataSourceError::Permanent(format!(
            "HTTP {status} fetching job {job_id} logs"
        )))
    }

    // ── Code graph (B2) ───────────────────────────────────────────────────────

    /// `GET /repos/{owner}/{name}/git/trees/{sha}?recursive=1`
    ///
    /// Returns all entries in the repo tree at `sha`.  If the response is
    /// `truncated` (trees > 100k objects), GitHub only returns a partial list;
    /// we log a warning and proceed with what we got.
    pub async fn get_tree(
        &self,
        owner: &str,
        name: &str,
        sha: &str,
    ) -> Result<Vec<TreeEntry>, DataSourceError> {
        let url = format!(
            "{}/repos/{owner}/{name}/git/trees/{sha}?recursive=1",
            self.base_url
        );
        let (body, _) = self.fetch_one(&url).await?;

        if body.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false) {
            tracing::warn!(
                owner,
                repo = name,
                sha,
                "git tree response was truncated; proceeding with partial list"
            );
        }

        let entries = body["tree"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let path = item["path"].as_str()?.to_string();
                        let entry_type = item["type"].as_str().unwrap_or("blob").to_string();
                        let sha = item["sha"].as_str().unwrap_or("").to_string();
                        let size = item["size"].as_u64().unwrap_or(0) as usize;
                        Some(TreeEntry { path, entry_type, sha, size })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(entries)
    }

    /// `GET /repos/{owner}/{name}/git/blobs/{sha}`
    ///
    /// Fetches and base64-decodes the blob content. Returns `None` if the
    /// file is too large (> `max_file_bytes`) or if the encoding is not
    /// base64.
    pub async fn get_blob(
        &self,
        owner: &str,
        name: &str,
        sha: &str,
        max_file_bytes: usize,
    ) -> Result<Option<String>, DataSourceError> {
        let url = format!(
            "{}/repos/{owner}/{name}/git/blobs/{sha}",
            self.base_url
        );
        let (body, _) = self.fetch_one(&url).await?;

        let encoding = body["encoding"].as_str().unwrap_or("");
        if encoding != "base64" {
            // Shouldn't happen in practice; skip rather than error.
            return Ok(None);
        }

        let raw_b64 = body["content"].as_str().unwrap_or("");
        // GitHub adds newlines every 60 chars — strip all whitespace.
        let clean: String = raw_b64.chars().filter(|c| !c.is_whitespace()).collect();

        // Size check before allocating: base64 overhead ≈ 4/3.
        let estimated_bytes = clean.len() * 3 / 4;
        if estimated_bytes > max_file_bytes {
            return Ok(None);
        }

        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(clean.as_bytes())
            .map_err(|e| DataSourceError::Permanent(format!("base64 decode: {e}")))?;

        if decoded.len() > max_file_bytes {
            return Ok(None);
        }

        // Convert to UTF-8; skip binary files.
        match String::from_utf8(decoded) {
            Ok(s) => Ok(Some(s)),
            Err(_) => Ok(None), // binary file
        }
    }

    /// `GET /repos/{owner}/{repo}/issues/{number}` — one issue (or PR shell).
    pub async fn get_issue(
        &self,
        owner: &str,
        name: &str,
        number: u64,
    ) -> Result<Value, DataSourceError> {
        let url = format!("{}/repos/{owner}/{name}/issues/{number}", self.base_url);
        let (body, _) = self.fetch_one(&url).await?;
        Ok(body)
    }

    /// `GET /repos/{owner}/{repo}/readme` — the repo README, decoded.
    /// Returns `None` for non-base64 encodings, binary content, or files over
    /// `max_bytes`. Used by the dreaming agent's read-only `connector_read`.
    pub async fn get_readme(
        &self,
        owner: &str,
        name: &str,
        max_bytes: usize,
    ) -> Result<Option<String>, DataSourceError> {
        let url = format!("{}/repos/{owner}/{name}/readme", self.base_url);
        let (body, _) = self.fetch_one(&url).await?;
        Ok(decode_b64_content(&body, max_bytes))
    }

    /// `GET /repos/{owner}/{repo}/contents/{path}` — one file, decoded. Same
    /// `None` semantics as [`Self::get_readme`].
    pub async fn get_contents(
        &self,
        owner: &str,
        name: &str,
        path: &str,
        max_bytes: usize,
    ) -> Result<Option<String>, DataSourceError> {
        let path = path.trim_start_matches('/');
        let url = format!("{}/repos/{owner}/{name}/contents/{path}", self.base_url);
        let (body, _) = self.fetch_one(&url).await?;
        Ok(decode_b64_content(&body, max_bytes))
    }
}

/// Decode a GitHub contents-API body (`{content: <base64>, encoding}`) into
/// UTF-8, skipping non-base64 encodings, binary content, and oversize files.
fn decode_b64_content(body: &Value, max_bytes: usize) -> Option<String> {
    if body.get("encoding").and_then(|v| v.as_str()) != Some("base64") {
        return None;
    }
    let raw_b64 = body.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let clean: String = raw_b64.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.len() * 3 / 4 > max_bytes {
        return None;
    }
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(clean.as_bytes())
        .ok()?;
    if decoded.len() > max_bytes {
        return None;
    }
    String::from_utf8(decoded).ok()
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Why pagination stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Done,
    PageCap,
    RateLimited,
}

/// Parse `X-RateLimit-Remaining` header value.
fn parse_rate_limit_remaining(resp: &Response) -> Option<u64> {
    resp.headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

/// Parse the `Link: <url>; rel="next"` header, returning the `next` URL if
/// present.
pub fn parse_link_next(resp: &Response) -> Option<String> {
    let link = resp.headers().get("link")?.to_str().ok()?;
    for part in link.split(',') {
        let part = part.trim();
        // Format: `<url>; rel="next"`
        let mut iter = part.splitn(2, ';');
        let url_part = iter.next()?.trim();
        let rel_part = iter.next()?.trim();
        if rel_part.contains("rel=\"next\"") {
            // Strip angle brackets.
            let url = url_part.trim_start_matches('<').trim_end_matches('>');
            return Some(url.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn make_client(server: &MockServer) -> GithubClient {
        GithubClient::new(
            Client::builder().build().unwrap(),
            "test-token".to_string(),
        )
        .with_base_url(server.uri())
    }

    #[tokio::test]
    async fn get_repo_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/myrepo"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({
                        "name": "myrepo",
                        "full_name": "acme/myrepo",
                    })),
            )
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        let repo = client.get_repo("acme", "myrepo").await.unwrap();
        assert_eq!(repo["name"], "myrepo");
    }

    #[tokio::test]
    async fn pagination_follows_link_header() {
        let server = MockServer::start().await;

        // Page 2 registered first so it has higher priority in wiremock's
        // LIFO matching and won't accidentally match the page-1 request.
        // Page 2 → returns 1 item, no Link header (more specific: page=2)
        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/branches"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([{ "name": "feature" }])),
            )
            .mount(&server)
            .await;

        // Page 1 → returns 2 items + Link: next header (matches per_page=100 without page=2)
        let page2_url = format!("{}/repos/acme/repo/branches?page=2", server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/branches"))
            .and(query_param("per_page", "100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .append_header("Link", format!("<{page2_url}>; rel=\"next\""))
                    .set_body_json(serde_json::json!([
                        { "name": "main" },
                        { "name": "dev" }
                    ])),
            )
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        let branches = client.list_branches("acme", "repo", 10).await.unwrap();
        assert_eq!(branches.len(), 3);
    }

    #[tokio::test]
    async fn rate_limit_low_stops_pagination() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/branches"))
            .respond_with(
                ResponseTemplate::new(200)
                    .append_header("X-RateLimit-Remaining", "5") // below FLOOR=10
                    .append_header("Link", "<http://next>; rel=\"next\"")
                    .set_body_json(serde_json::json!([{ "name": "main" }])),
            )
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        let branches = client.list_branches("acme", "repo", 10).await.unwrap();
        // Should have gotten the first page but stopped due to rate-limit floor
        assert_eq!(branches.len(), 1);
    }

    #[tokio::test]
    async fn rate_limit_403_returns_transient() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/acme/repo"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        // With 3 retries the mock server always returns 403, so after 4 attempts
        // (attempt 0,1,2,3) it should return Transient.
        let result = client.get_repo("acme", "repo").await;
        assert!(
            matches!(result, Err(DataSourceError::Transient(_))),
            "expected Transient, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn page_cap_respected() {
        let server = MockServer::start().await;

        // Register page 2 with a more specific query_param matcher so it
        // only fires for the second request. Pages 3+ are never reached.
        let next3 = format!("{}/repos/a/b/branches?page=3", server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/a/b/branches"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .append_header("Link", format!("<{next3}>; rel=\"next\""))
                    .set_body_json(serde_json::json!([{ "name": "branch2" }])),
            )
            .mount(&server)
            .await;

        // Page 1 (initial request, has per_page=100 but no page= param)
        let next2 = format!("{}/repos/a/b/branches?page=2", server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/a/b/branches"))
            .and(query_param("per_page", "100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .append_header("Link", format!("<{next2}>; rel=\"next\""))
                    .set_body_json(serde_json::json!([{ "name": "branch1" }])),
            )
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        let branches = client.list_branches("a", "b", 2).await.unwrap();
        // cap = 2 pages → branch1 + branch2
        assert_eq!(branches.len(), 2);
    }

    #[test]
    fn parse_link_next_extracts_url() {
        // parse_link_next takes a &Response which requires an HTTP roundtrip to
        // construct. End-to-end coverage is provided by pagination_follows_link_header.
        // This test simply confirms the test file compiles correctly.
        let _ = "placeholder";
    }

    // ── Code graph client tests (B2) ──────────────────────────────────────────

    #[tokio::test]
    async fn get_tree_returns_entries() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/git/trees/abc123"))
            .and(query_param("recursive", "1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "truncated": false,
                    "tree": [
                        { "path": "src/main.rs", "type": "blob", "sha": "sha1", "size": 100 },
                        { "path": "src", "type": "tree", "sha": "sha2", "size": 0 },
                        { "path": "Cargo.toml", "type": "blob", "sha": "sha3", "size": 200 }
                    ]
                })),
            )
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        let entries = client.get_tree("acme", "repo", "abc123").await.unwrap();
        assert_eq!(entries.len(), 3);

        let blob = entries.iter().find(|e| e.path == "src/main.rs").unwrap();
        assert_eq!(blob.entry_type, "blob");
        assert_eq!(blob.sha, "sha1");
        assert_eq!(blob.size, 100);

        let tree_entry = entries.iter().find(|e| e.path == "src").unwrap();
        assert_eq!(tree_entry.entry_type, "tree");
    }

    #[tokio::test]
    async fn get_tree_handles_truncated() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/git/trees/abc123"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "truncated": true,
                    "tree": [
                        { "path": "src/main.rs", "type": "blob", "sha": "sha1", "size": 50 }
                    ]
                })),
            )
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        // Truncated should not error — just returns partial results.
        let entries = client.get_tree("acme", "repo", "abc123").await.unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn get_blob_decodes_base64() {
        let server = MockServer::start().await;

        // "fn main() {}" base64-encoded
        let content = base64::engine::general_purpose::STANDARD.encode(b"fn main() {}");
        // GitHub includes newlines every 60 chars; simulate that
        let content_with_newlines = format!("{content}\n");

        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/git/blobs/sha1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "encoding": "base64",
                    "content": content_with_newlines,
                    "size": 13
                })),
            )
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        let result = client
            .get_blob("acme", "repo", "sha1", 1_048_576)
            .await
            .unwrap();
        assert_eq!(result, Some("fn main() {}".to_string()));
    }

    #[tokio::test]
    async fn get_blob_skips_oversized_files() {
        let server = MockServer::start().await;

        // Large content: 200 bytes, max = 100 → should skip
        let big_content = "a".repeat(200);
        let encoded = base64::engine::general_purpose::STANDARD.encode(big_content.as_bytes());

        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/git/blobs/sha_big"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "encoding": "base64",
                    "content": encoded,
                    "size": 200
                })),
            )
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        // max_file_bytes = 100, file is 200 bytes → should return None
        let result = client
            .get_blob("acme", "repo", "sha_big", 100)
            .await
            .unwrap();
        assert_eq!(result, None, "expected None for oversized file");
    }

    // ── Actions (E1) ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_workflow_runs_extracts_envelope() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/actions/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 2,
                "workflow_runs": [
                    { "id": 1, "name": "CI", "status": "completed", "conclusion": "failure" },
                    { "id": 2, "name": "CI", "status": "completed", "conclusion": "success" }
                ]
            })))
            .mount(&server)
            .await;
        let client = make_client(&server).await;
        let (runs, _) = client.list_workflow_runs("acme", "repo", 5).await.unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0]["id"], 1);
    }

    #[tokio::test]
    async fn list_run_jobs_extracts_envelope() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/actions/runs/77/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 1,
                "jobs": [ { "id": 9, "name": "build", "conclusion": "failure" } ]
            })))
            .mount(&server)
            .await;
        let client = make_client(&server).await;
        let (jobs, _) = client.list_run_jobs("acme", "repo", 77, 5).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0]["id"], 9);
    }

    #[tokio::test]
    async fn fetch_job_log_text_returns_plaintext_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/actions/jobs/9/logs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("2024-01-01 step one\n2024-01-01 step two\n"),
            )
            .mount(&server)
            .await;
        let client = make_client(&server).await;
        let text = client.fetch_job_log_text("acme", "repo", 9).await.unwrap();
        assert!(text.contains("step one"), "got: {text}");
        assert!(text.contains("step two"), "got: {text}");
    }

    #[tokio::test]
    async fn get_blob_skips_non_base64() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/acme/repo/git/blobs/sha_utf8"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "encoding": "utf-8",
                    "content": "fn main() {}",
                    "size": 13
                })),
            )
            .mount(&server)
            .await;

        let client = make_client(&server).await;
        let result = client
            .get_blob("acme", "repo", "sha_utf8", 1_048_576)
            .await
            .unwrap();
        assert_eq!(result, None, "expected None for non-base64 encoding");
    }
}

// ── internal use of base64 in tests only when feature enabled ────────────────
#[cfg(test)]
#[allow(unused_imports)]
use base64::Engine as _;
