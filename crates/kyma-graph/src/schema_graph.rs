//! The synthetic schema-graph: the catalog rendered as a property-graph.

use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::provider::GraphProvider;
use crate::source::{ColumnDef, SchemaSource};
use crate::types::{
    Direction, EdgeExpansion, GraphNode, GraphPayload, GraphRelationship, GraphSchema, GraphStats,
    NodeMetadata, Props, SearchHits,
};

/// Stable node id for a table.
fn table_node_id(database: &str, table: &str) -> String {
    format!("{database}::{table}")
}

/// Infer `REFERENCES` edges among tables of one database from `<base>_id`
/// column names. Pure + deterministic so it is trivially testable.
pub(crate) fn infer_edges(
    database: &str,
    tables: &[(String, Vec<ColumnDef>)],
) -> Vec<GraphRelationship> {
    let names: Vec<String> = tables.iter().map(|(n, _)| n.to_lowercase()).collect();
    let mut edges = Vec::new();
    for (tname, cols) in tables {
        for c in cols {
            let lname = c.name.to_lowercase();
            let Some(base) = lname.strip_suffix("_id") else { continue };
            if base.is_empty() {
                continue;
            }
            // Match a table whose name is `base` or `base` + "s".
            let target = names.iter().find(|n| *n == base || **n == format!("{base}s"));
            if let Some(target_lc) = target {
                let target_name = tables
                    .iter()
                    .find(|(n, _)| n.to_lowercase() == *target_lc)
                    .map(|(n, _)| n.clone())
                    .unwrap();
                if target_name == *tname {
                    continue; // no self-edge
                }
                let mut props: Props = BTreeMap::new();
                props.insert("via".into(), serde_json::json!(c.name));
                edges.push(GraphRelationship {
                    id: format!(
                        "{}->{}:{}",
                        table_node_id(database, tname),
                        table_node_id(database, &target_name),
                        c.name
                    ),
                    source_id: table_node_id(database, tname),
                    target_id: table_node_id(database, &target_name),
                    relationship_type: "REFERENCES".into(),
                    properties: props,
                });
            }
        }
    }
    edges
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    use crate::source::ColumnDef;

    fn col(name: &str) -> ColumnDef {
        ColumnDef { name: name.into(), type_: "string".into(), nullable: true }
    }

    #[test]
    fn infers_fk_edge_from_user_id_to_users() {
        let tables = vec![
            ("users".to_string(), vec![col("id"), col("email")]),
            ("orders".to_string(), vec![col("id"), col("user_id"), col("total")]),
        ];
        let edges = infer_edges("default", &tables);
        assert_eq!(edges.len(), 1);
        let e = &edges[0];
        assert_eq!(e.source_id, "default::orders");
        assert_eq!(e.target_id, "default::users");
        assert_eq!(e.relationship_type, "REFERENCES");
        assert_eq!(e.properties["via"], "user_id");
    }

    #[test]
    fn no_edge_when_no_matching_table() {
        let tables = vec![
            ("orders".to_string(), vec![col("id"), col("customer_id")]),
        ];
        assert!(infer_edges("default", &tables).is_empty());
    }

    #[test]
    fn plain_id_column_is_not_an_edge() {
        let tables = vec![("users".to_string(), vec![col("id")])];
        assert!(infer_edges("default", &tables).is_empty());
    }
}
