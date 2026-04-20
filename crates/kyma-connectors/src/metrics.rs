//! Per-tick metrics helpers. Implementation lands in Task 6.

#[derive(Clone, Debug)]
pub struct ConnectorMetrics {
    pub connector_id: uuid::Uuid,
    pub type_id: &'static str,
}
