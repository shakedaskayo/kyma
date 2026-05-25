//! GitHub connector admin endpoints.
//!
//! Exposes `POST /v1/connectors/github/repos` — resolves the provided PAT
//! via `SecretStore`, lists all repos visible to the token (user + org repos),
//! and returns them for the UI's repo picker.
//!
//! This router must be mounted behind `Role::Write` auth middleware.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::secrets::SecretStore;

use super::client::GithubClient;

const DEFAULT_MAX_PAGES: usize = 10;

#[derive(Deserialize)]
struct ListReposReq {
    /// SecretStore ref (e.g. `"$env:GITHUB_TOKEN"`) or raw PAT.
    token: String,
}

#[derive(Serialize)]
struct RepoItem {
    full_name: String,
    name: String,
    owner: String,
    private: bool,
    default_branch: String,
    description: Option<String>,
}

#[derive(Serialize)]
struct ListReposResp {
    repos: Vec<RepoItem>,
}

#[derive(Clone)]
struct AdminGithubState {
    secrets: Arc<dyn SecretStore>,
}

pub fn github_repos_router(secrets: Arc<dyn SecretStore>) -> Router {
    let state = AdminGithubState { secrets };
    Router::new()
        .route("/v1/connectors/github/repos", post(list_repos))
        .with_state(state)
}

async fn list_repos(
    axum::extract::State(state): axum::extract::State<AdminGithubState>,
    Json(req): Json<ListReposReq>,
) -> impl IntoResponse {
    // Resolve the token (may be a $env: reference or a raw PAT).
    let token = match state.secrets.resolve(&req.token) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("token resolve: {e}") })),
            )
                .into_response();
        }
    };

    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("http client: {e}") })),
            )
                .into_response();
        }
    };

    let client = GithubClient::new(http, token);

    match client.list_user_repos(DEFAULT_MAX_PAGES).await {
        Ok(raw_repos) => {
            let repos: Vec<RepoItem> = raw_repos
                .into_iter()
                .filter_map(|r| {
                    let full_name = r["full_name"].as_str()?.to_string();
                    let name = r["name"].as_str()?.to_string();
                    let owner = r["owner"]["login"].as_str()?.to_string();
                    let private = r["private"].as_bool().unwrap_or(false);
                    let default_branch = r["default_branch"]
                        .as_str()
                        .unwrap_or("main")
                        .to_string();
                    let description = r["description"].as_str().map(|s| s.to_string());
                    Some(RepoItem {
                        full_name,
                        name,
                        owner,
                        private,
                        default_branch,
                        description,
                    })
                })
                .collect();
            (StatusCode::OK, Json(ListReposResp { repos })).into_response()
        }
        Err(e) => {
            let status = match &e {
                crate::types::ConnectorError::Transient(_) => StatusCode::BAD_GATEWAY,
                _ => StatusCode::BAD_REQUEST,
            };
            (
                status,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    }
}
