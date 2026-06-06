//! Per-database authorization. `Principal.allowed_databases == None` means
//! unrestricted (all pre-OIDC backends). Handlers call this at the moment
//! they resolve which database a request targets.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::backend::{Principal, Role};

    fn principal_unrestricted() -> Principal {
        Principal {
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            role: Role::Read,
            subject: None,
            allowed_databases: None,
        }
    }

    fn principal_scoped(dbs: &[&str]) -> Principal {
        Principal {
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            role: Role::Read,
            subject: None,
            allowed_databases: Some(dbs.iter().map(|s| s.to_string()).collect()),
        }
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
}
