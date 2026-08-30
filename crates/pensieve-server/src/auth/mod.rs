//! Pluggable bearer-token authentication.

mod backend;
mod env_backend;
mod git_middleware;
mod middleware;
pub mod oidc_backend;
pub mod passwords;
pub mod scope;
pub mod session_backend;
pub mod supabase_backend;

#[cfg(feature = "cloud-auth")]
mod db_backend;

pub use backend::{AuthBackend, AuthError, Principal, Role};
pub use env_backend::EnvAuthBackend;
pub use git_middleware::require_git_auth_middleware;
pub use middleware::{require_role_middleware, AuthLayerState};
pub use oidc_backend::{OidcAuthBackend, OidcConfig};
pub use scope::{
    check_database_scope, check_realm_scope, intersect_realms, is_memory_internal_db,
    EffectiveRealms, RealmScope,
};
pub use session_backend::{hash_token, SessionAuthBackend};
pub use supabase_backend::{SupabaseAuthBackend, SupabaseAuthConfig};

#[cfg(feature = "cloud-auth")]
pub use db_backend::DbAuthBackend;

// Backwards-compat re-export. New code should use `EnvAuthBackend` or
// `Arc<dyn AuthBackend>` directly.
#[deprecated(note = "use EnvAuthBackend or Arc<dyn AuthBackend> directly")]
pub use env_backend::EnvAuthBackend as AuthConfig;
