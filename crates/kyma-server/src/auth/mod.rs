//! Pluggable bearer-token authentication.

mod backend;
mod env_backend;
mod middleware;
pub mod passwords;
pub mod session_backend;
pub mod supabase_backend;

#[cfg(feature = "cloud-auth")]
mod db_backend;

pub use backend::{AuthBackend, AuthError, Principal, Role};
pub use env_backend::EnvAuthBackend;
pub use middleware::{require_role_middleware, AuthLayerState};
pub use session_backend::{hash_token, SessionAuthBackend};
pub use supabase_backend::{SupabaseAuthBackend, SupabaseAuthConfig};

#[cfg(feature = "cloud-auth")]
pub use db_backend::DbAuthBackend;

// Backwards-compat re-export. New code should use `EnvAuthBackend` or
// `Arc<dyn AuthBackend>` directly.
#[deprecated(note = "use EnvAuthBackend or Arc<dyn AuthBackend> directly")]
pub use env_backend::EnvAuthBackend as AuthConfig;
