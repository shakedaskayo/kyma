//! Tenant identity type — load-bearing for cloud's multi-workspace isolation.
//!
//! Every catalog row, every storage object, and every authenticated request
//! carries a [`TenantId`]. Self-hosted deployments use [`DEFAULT_TENANT`];
//! cloud deployments mint a fresh UUID per workspace.

use std::str::FromStr;
use uuid::Uuid;

/// Identifier of a tenant (≅ a cloud workspace, or the single self-hosted owner).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TenantId(Uuid);

impl TenantId {
    pub const fn from_uuid(u: Uuid) -> Self { Self(u) }
    pub const fn as_uuid(self) -> Uuid { self.0 }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for TenantId {
    fn default() -> Self { DEFAULT_TENANT }
}

impl FromStr for TenantId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// All-zero UUID used by self-hosted deployments and as the migration default.
pub const DEFAULT_TENANT: TenantId = TenantId::from_uuid(Uuid::nil());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tenant_is_all_zeros() {
        assert_eq!(DEFAULT_TENANT.as_uuid().as_bytes(), &[0u8; 16]);
    }

    #[test]
    fn tenant_id_displays_as_hyphenated_uuid() {
        let t = DEFAULT_TENANT;
        assert_eq!(t.to_string(), "00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn parses_from_uuid_string() {
        let t: TenantId = "11111111-1111-1111-1111-111111111111".parse().unwrap();
        assert_eq!(t.to_string(), "11111111-1111-1111-1111-111111111111");
    }

    #[test]
    fn default_returns_default_tenant() {
        assert_eq!(TenantId::default(), DEFAULT_TENANT);
    }
}
