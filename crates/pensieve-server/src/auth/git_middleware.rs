//! Auth middleware for the `/git` smart-HTTP subtree. Git clients speak
//! HTTP Basic, not Bearer: the pensieve API token rides as the Basic *password*
//! (username is ignored — `git clone http://pensieve:<token>@host/...`) or as
//! the username when the password is empty. Bearer is also accepted so
//! non-git clients can hit the same routes.
//!
//! The middleware only authenticates (minimum `Role::Read`); per-operation
//! role enforcement (clone vs push, per-brain visibility) happens in the
//! git handlers, which know which service is being invoked.

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine as _;

use super::backend::{AuthError, Role};
use super::middleware::AuthLayerState;

fn basic_unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, r#"Basic realm="pensieve""#)],
        msg.to_owned(),
    )
        .into_response()
}

/// Extract a pensieve token from `Authorization: Basic …` or `Bearer …`.
fn extract_token(value: &str) -> Option<String> {
    if let Some(bearer) = value.strip_prefix("Bearer ") {
        return Some(bearer.trim().to_string());
    }
    let b64 = value.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (user, pass) = text.split_once(':').unwrap_or((text.as_str(), ""));
    let token = if pass.is_empty() { user } else { pass };
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

pub async fn require_git_auth_middleware(
    State(state): State<AuthLayerState>,
    mut req: Request,
    next: Next,
) -> Response {
    let principal = if state.backend.enabled() {
        let token = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(extract_token);
        let Some(token) = token else {
            return basic_unauthorized("authentication required");
        };
        match state.backend.authenticate(&token).await {
            Ok(p) if p.role >= state.required => p,
            // Valid credentials but insufficient role → 403, not 401 (a 401
            // would loop git's credential prompt).
            Ok(_) => return (StatusCode::FORBIDDEN, "token role insufficient").into_response(),
            Err(AuthError::UnknownToken | AuthError::MissingToken) => {
                return basic_unauthorized("unknown token");
            }
            Err(AuthError::Backend(e)) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("auth backend: {e}"))
                    .into_response();
            }
        }
    } else {
        super::backend::Principal {
            tenant: pensieve_core::tenant::DEFAULT_TENANT,
            role: Role::Admin,
            subject: None,
            allowed_databases: None,
            allowed_realms: None,
        }
    };
    let tenant = principal.tenant;
    req.extensions_mut().insert(principal);
    req.extensions_mut().insert(tenant);
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::extract_token;
    use base64::Engine as _;

    fn basic(user: &str, pass: &str) -> String {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
        )
    }

    #[test]
    fn token_from_password_username_or_bearer() {
        assert_eq!(extract_token(&basic("pensieve", "tok1")).as_deref(), Some("tok1"));
        assert_eq!(extract_token(&basic("tok2", "")).as_deref(), Some("tok2"));
        assert_eq!(extract_token("Bearer tok3").as_deref(), Some("tok3"));
        assert_eq!(extract_token(&basic("", "")), None);
        assert_eq!(extract_token("Digest abc"), None);
    }
}
