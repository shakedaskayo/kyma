//! Per-database and per-realm authorization. `Principal.allowed_databases`
//! / `Principal.allowed_realms == None` means unrestricted (all pre-OIDC
//! backends). Handlers call these at the moment they resolve which database
//! or memory realm a request targets.

use super::backend::Principal;
use axum::http::StatusCode;

pub fn check_database_scope(
    principal: &Principal,
    database: &str,
) -> Result<(), (StatusCode, String)> {
    match &principal.allowed_databases {
        None => Ok(()),
        Some(allowed) if allowed.iter().any(|d| d == database) => Ok(()),
        Some(_) => Err((
            StatusCode::FORBIDDEN,
            format!("token not scoped to database `{database}`"),
        )),
    }
}

// ── Realm scoping ────────────────────────────────────────────────────────
//
// A realm-scoped token restricts every memory read/write to a fixed set of
// realms. `None` = unrestricted (identical to today for every existing
// token). Enforcement is layered above the SQL builders (which are
// fail-open on an empty realm list) — see `intersect_realms`, whose
// `Scoped` variant is guaranteed non-empty so no restricted read ever
// widens to "all realms".

/// The realm allow-list carried by a [`Principal`], lifted into a small value
/// type so tool contexts can hold it without depending on the auth module's
/// larger `Principal`.
#[derive(Debug, Clone, Default)]
pub struct RealmScope(pub Option<Vec<String>>);

impl RealmScope {
    pub fn unrestricted() -> Self {
        Self(None)
    }
    pub fn restricted(realms: Vec<String>) -> Self {
        Self(Some(realms))
    }
    pub fn from_principal(p: &Principal) -> Self {
        Self(p.allowed_realms.clone())
    }
    pub fn is_restricted(&self) -> bool {
        self.0.is_some()
    }
    /// Whether this scope permits touching `realm`. Unrestricted ⇒ always
    /// true. An empty allow-list (`Some(vec![])`, from an empty `pensieve_realms`
    /// claim) permits nothing — consistently fail-closed.
    pub fn allows(&self, realm: &str) -> bool {
        match &self.0 {
            None => true,
            Some(list) => list.iter().any(|r| r == realm),
        }
    }
}

/// Effective realm filter for a read, after applying the token scope to the
/// caller-requested realms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveRealms {
    /// Unrestricted token: use the requested list verbatim (empty still means
    /// "all realms", matching today's behavior).
    Unrestricted(Vec<String>),
    /// Restricted token: a **non-empty** list to place in
    /// `RecallFilter.realms` (requested empty → the full allowed list;
    /// otherwise requested ∩ allowed). Non-emptiness is the invariant that
    /// keeps `filter_conditions` (fail-open on empty) from widening a
    /// restricted read.
    Scoped(Vec<String>),
    /// requested ∩ allowed = ∅ → run nothing, return zero results.
    Empty,
}

/// Intersect the caller-requested realms with the token's allow-list.
///
/// The critical correctness point (vs. a naive filter): when a restricted
/// caller requests *no* realms, the effective set is the **allowed** list,
/// never the empty "all realms" set.
pub fn intersect_realms(scope: &RealmScope, requested: &[String]) -> EffectiveRealms {
    match &scope.0 {
        None => EffectiveRealms::Unrestricted(requested.to_vec()),
        Some(allowed) => {
            let eff: Vec<String> = if requested.is_empty() {
                allowed.clone()
            } else {
                requested
                    .iter()
                    .filter(|r| allowed.iter().any(|a| a == *r))
                    .cloned()
                    .collect()
            };
            if eff.is_empty() {
                EffectiveRealms::Empty
            } else {
                EffectiveRealms::Scoped(eff)
            }
        }
    }
}

/// Databases holding memory-internal tables. A realm-restricted token must not
/// reach these through the generic query/search surfaces, which see every
/// realm regardless of any realm predicate.
pub fn is_memory_internal_db(db: &str) -> bool {
    db == pensieve_memory::DEFAULT_DATABASE
        || db == pensieve_memory::file_candidates::FILE_CANDIDATES_DB
        || db == pensieve_memory::activities::ACTIVITIES_DB
}

/// HTTP-flavored realm membership check, mirroring [`check_database_scope`].
pub fn check_realm_scope(
    principal: &Principal,
    realm: &str,
) -> Result<(), (StatusCode, String)> {
    match &principal.allowed_realms {
        None => Ok(()),
        Some(allowed) if allowed.iter().any(|r| r == realm) => Ok(()),
        Some(_) => Err((
            StatusCode::FORBIDDEN,
            format!("token not scoped to realm `{realm}`"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::backend::{Principal, Role};

    fn principal_unrestricted() -> Principal {
        Principal {
            tenant: pensieve_core::tenant::DEFAULT_TENANT,
            role: Role::Read,
            subject: None,
            allowed_databases: None,
            allowed_realms: None,
        }
    }

    fn principal_scoped(dbs: &[&str]) -> Principal {
        Principal {
            tenant: pensieve_core::tenant::DEFAULT_TENANT,
            role: Role::Read,
            subject: None,
            allowed_databases: Some(dbs.iter().map(|s| s.to_string()).collect()),
            allowed_realms: None,
        }
    }

    fn principal_realm_scoped(realms: &[&str]) -> Principal {
        Principal {
            tenant: pensieve_core::tenant::DEFAULT_TENANT,
            role: Role::Read,
            subject: None,
            allowed_databases: None,
            allowed_realms: Some(realms.iter().map(|s| s.to_string()).collect()),
        }
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unrestricted_allows_everything() {
        let p = principal_unrestricted();
        assert!(check_database_scope(&p, "prod").is_ok());
        assert!(check_database_scope(&p, "anything").is_ok());
        assert!(check_database_scope(&p, "default").is_ok());
    }

    #[test]
    fn scoped_allows_listed() {
        let p = principal_scoped(&["prod", "staging"]);
        assert!(check_database_scope(&p, "prod").is_ok());
        assert!(check_database_scope(&p, "staging").is_ok());
    }

    #[test]
    fn scoped_rejects_unlisted() {
        let p = principal_scoped(&["prod"]);
        let err = check_database_scope(&p, "other_db").unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert!(err.1.contains("other_db"));
    }

    // ── Realm scope ──────────────────────────────────────────────────────

    #[test]
    fn realm_scope_allows() {
        assert!(RealmScope::unrestricted().allows("anything"));
        let s = RealmScope::restricted(v(&["proj", "global"]));
        assert!(s.allows("proj"));
        assert!(s.allows("global"));
        assert!(!s.allows("other"));
        // Empty allow-list permits nothing (fail-closed).
        assert!(!RealmScope::restricted(vec![]).allows("proj"));
    }

    #[test]
    fn intersect_unrestricted_passthrough() {
        let s = RealmScope::unrestricted();
        assert_eq!(
            intersect_realms(&s, &[]),
            EffectiveRealms::Unrestricted(vec![])
        );
        assert_eq!(
            intersect_realms(&s, &v(&["a", "b"])),
            EffectiveRealms::Unrestricted(v(&["a", "b"]))
        );
    }

    #[test]
    fn intersect_restricted_empty_request_yields_allowed() {
        // The critical case: empty requested must resolve to the ALLOWED
        // list, never the empty "all realms" set.
        let s = RealmScope::restricted(v(&["proj", "global"]));
        assert_eq!(
            intersect_realms(&s, &[]),
            EffectiveRealms::Scoped(v(&["proj", "global"]))
        );
    }

    #[test]
    fn intersect_restricted_overlap() {
        let s = RealmScope::restricted(v(&["proj", "global"]));
        assert_eq!(
            intersect_realms(&s, &v(&["proj", "other"])),
            EffectiveRealms::Scoped(v(&["proj"]))
        );
    }

    #[test]
    fn intersect_restricted_disjoint_is_empty() {
        let s = RealmScope::restricted(v(&["proj"]));
        assert_eq!(intersect_realms(&s, &v(&["other"])), EffectiveRealms::Empty);
        // Empty allow-list ⇒ Empty for any request.
        let s0 = RealmScope::restricted(vec![]);
        assert_eq!(intersect_realms(&s0, &[]), EffectiveRealms::Empty);
        assert_eq!(intersect_realms(&s0, &v(&["x"])), EffectiveRealms::Empty);
    }

    #[test]
    fn check_realm_scope_three_cases() {
        let unrestricted = principal_unrestricted();
        assert!(check_realm_scope(&unrestricted, "anything").is_ok());

        let scoped = principal_realm_scoped(&["proj"]);
        assert!(check_realm_scope(&scoped, "proj").is_ok());
        let err = check_realm_scope(&scoped, "other").unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert!(err.1.contains("other"));
    }

    #[test]
    fn memory_internal_db_detection() {
        assert!(is_memory_internal_db("memory"));
        assert!(is_memory_internal_db("file_candidates"));
        assert!(is_memory_internal_db("activities"));
        assert!(!is_memory_internal_db("default"));
        assert!(!is_memory_internal_db("otel"));
    }
}
