//! Pluggable bearer-token authentication.

mod backend;
mod env_backend;
mod middleware;
pub mod oidc_backend;
pub mod passwords;
pub mod session_backend;

#[cfg(feature = "cloud-auth")]
mod db_backend;

pub use backend::{AuthBackend, AuthError, Principal, Role};
pub use env_backend::EnvAuthBackend;
pub use middleware::{require_role_middleware, AuthLayerState};
pub use oidc_backend::{OidcAuthBackend, OidcConfig};
pub use session_backend::{hash_token, SessionAuthBackend};

#[cfg(feature = "cloud-auth")]
pub use db_backend::DbAuthBackend;

// Backwards-compat re-export. New code should use `EnvAuthBackend` or
// `Arc<dyn AuthBackend>` directly.
#[deprecated(note = "use EnvAuthBackend or Arc<dyn AuthBackend> directly")]
pub use env_backend::EnvAuthBackend as AuthConfig;
