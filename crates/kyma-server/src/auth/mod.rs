//! Pluggable bearer-token authentication.

mod backend;
mod env_backend;
mod middleware;
pub mod passwords;

#[cfg(feature = "cloud-auth")]
mod db_backend;

pub use backend::{AuthBackend, AuthError, Principal, Role};
pub use env_backend::EnvAuthBackend;
pub use middleware::{require_role_middleware, AuthLayerState};

#[cfg(feature = "cloud-auth")]
pub use db_backend::DbAuthBackend;

// Backwards-compat re-export. New code should use `EnvAuthBackend` or
// `Arc<dyn AuthBackend>` directly.
#[deprecated(note = "use EnvAuthBackend or Arc<dyn AuthBackend> directly")]
pub use env_backend::EnvAuthBackend as AuthConfig;
