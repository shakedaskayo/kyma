//! Pluggable bearer-token authentication.

mod backend;
mod env_backend;
mod middleware;

#[cfg(feature = "cloud-auth")]
mod db_backend;

pub use backend::{AuthBackend, AuthError, Principal, Role};
pub use env_backend::EnvAuthBackend;
pub use middleware::{require_role_middleware, AuthLayerState};

#[cfg(feature = "cloud-auth")]
pub use db_backend::DbAuthBackend;

// Backwards-compat re-export for legacy `kyma-bin` callers.
pub use env_backend::EnvAuthBackend as AuthConfig;
