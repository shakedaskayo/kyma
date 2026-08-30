//! Adapter implementing the [`SavedViewLookup`] trait (declared in
//! [`super::scope`]) against the catalog's `saved_views` module.
//!
//! The lookup is owner-scoped: a saved view belongs to a single subject
//! within a tenant. A view-id that doesn't parse as a UUID, or that exists
//! in the table but not under this (`tenant`, `owner_subject`) tuple, both
//! resolve to `Ok(None)` so the caller surfaces a single
//! `ScopeError::ViewNotFound` regardless of cause.

use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use super::scope::SavedViewLookup;
use pensieve_catalog::saved_views;

/// Catalog-backed implementation of [`SavedViewLookup`] for a single
/// (`tenant_id`, `owner_subject`) caller.
pub struct CatalogSavedViewLookup {
    pub pool: Arc<PgPool>,
    pub tenant_id: Uuid,
    pub owner_subject: String,
}

#[async_trait::async_trait]
impl SavedViewLookup for CatalogSavedViewLookup {
    async fn load_sources(&self, view_id: &str) -> Result<Option<Vec<String>>, String> {
        let parsed = match Uuid::parse_str(view_id) {
            Ok(u) => u,
            Err(_) => return Ok(None),
        };
        match saved_views::get(&self.pool, self.tenant_id, &self.owner_subject, parsed).await {
            Ok(v) => Ok(Some(v.sources)),
            Err(saved_views::SavedViewError::NotFound) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}
