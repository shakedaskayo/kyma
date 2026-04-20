//! The `QueryFrontendRegistry` — routes queries to the right registered
//! frontend by Content-Type and unpacks the opaque plan payload into the
//! concrete [`LogicalPlan`][super::logical_plan::LogicalPlan].
//!
//! TODO(M2): implement registration + lookup + downcast.
