//! MCP tool dispatch wrapping the eight agent tool factories.

use adk_rust::tool::SimpleToolContext;
use adk_rust::Tool;
use kyma_server::agent::{
    tool_describe_table, tool_explore_schema, tool_find_references_to, tool_graph_traverse,
    tool_list_databases, tool_run_kql, tool_run_sql, tool_sample_rows, SharedToolCtx,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::jsonrpc::{ErrorCode, ErrorObject};

#[derive(Clone)]
pub struct ToolDispatch {
    by_name: Arc<HashMap<&'static str, Arc<dyn Tool>>>,
}

impl ToolDispatch {
    pub fn new(shared: SharedToolCtx) -> Self {
        let mut map: HashMap<&'static str, Arc<dyn Tool>> = HashMap::with_capacity(8);
        map.insert("list_databases", tool_list_databases(shared.clone()));
        map.insert("describe_table", tool_describe_table(shared.clone()));
        map.insert("run_sql", tool_run_sql(shared.clone()));
        map.insert("run_kql", tool_run_kql(shared.clone()));
        map.insert("sample_rows", tool_sample_rows(shared.clone()));
        map.insert("explore_schema", tool_explore_schema(shared.clone()));
        map.insert("find_references_to", tool_find_references_to(shared.clone()));
        map.insert("graph_traverse", tool_graph_traverse(shared));
        Self { by_name: Arc::new(map) }
    }

    /// Render the tools as MCP `tools/list` entries.
    pub fn list(&self) -> Vec<Value> {
        let mut entries: Vec<(&'static str, Value)> = Vec::with_capacity(self.by_name.len());
        for (name, tool) in self.by_name.iter() {
            let input_schema = tool
                .parameters_schema()
                .unwrap_or_else(|| json!({"type": "object"}));
            entries.push((
                *name,
                json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "inputSchema": input_schema,
                }),
            ));
        }
        entries.sort_by(|a, b| a.0.cmp(b.0));
        entries.into_iter().map(|(_, v)| v).collect()
    }

    /// Invoke a tool by name. Translates ADK errors into JSON-RPC errors.
    pub async fn call(&self, name: &str, arguments: Value) -> Result<Value, ErrorObject> {
        let Some(tool) = self.by_name.get(name).cloned() else {
            return Err(ErrorObject::new(
                ErrorCode::MethodNotFound as i64,
                format!("unknown tool: {name}"),
            ));
        };
        let ctx = Arc::new(SimpleToolContext::new("kyma-mcp"));
        match tool.execute(ctx, arguments).await {
            Ok(value) => Ok(json!({
                "content": [
                    {"type": "text", "text": serde_json::to_string(&value).unwrap_or_else(|_| "{}".into())}
                ],
                "isError": value.get("error").is_some(),
                "structuredContent": value,
            })),
            Err(e) => Err(ErrorObject::new(
                ErrorCode::InternalError as i64,
                format!("tool {name}: {e}"),
            )),
        }
    }
}
