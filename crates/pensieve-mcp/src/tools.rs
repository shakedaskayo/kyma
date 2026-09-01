//! MCP tool dispatch wrapping the agent query + memory tool factories.

use adk_rust::tool::SimpleToolContext;
use adk_rust::Tool;
use pensieve_server::agent::{
    tool_contribute_file, tool_describe_file, tool_describe_table, tool_explore_schema,
    tool_file_neighbors, tool_find_references_to, tool_flush_memory, tool_graph_analytics,
    tool_graph_search, tool_graph_traverse, tool_ingest_entity, tool_link_memory_to_entity,
    tool_list_databases, tool_list_memories, tool_list_memory_usage, tool_memory_compare,
    tool_memory_judge, tool_memory_search, tool_memory_session_summary, tool_recall_file,
    tool_recall_memory, tool_reinforce_memory, tool_retrieve_artifact, tool_run_kql,
    tool_run_sql, tool_sample_rows, tool_save_memories, tool_save_memory, tool_search,
    tool_update_memory_importance, tool_update_memory_status, SharedToolCtx,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::jsonrpc::{ErrorCode, ErrorObject};

/// Tools that can address arbitrary databases/realms and therefore bypass the
/// memory realm model (raw SQL/KQL, cross-realm graph traversal, unified
/// search, realm-blind file recall, catalog/datasource reads). They are NOT
/// registered for realm-restricted callers, and [`ToolDispatch::call`] names
/// the policy rather than returning "unknown tool".
pub const REALM_RESTRICTED_TOOLS: &[&str] = &[
    "list_databases",
    "describe_table",
    "run_sql",
    "run_kql",
    "sample_rows",
    "explore_schema",
    "find_references_to",
    "graph_traverse",
    "graph_analytics",
    "search",
    "graph_search",
    "recall_file",
    "retrieve_artifact",
    "list_data_sources",
    "data_source_read",
];

#[derive(Clone)]
pub struct ToolDispatch {
    by_name: Arc<HashMap<&'static str, Arc<dyn Tool>>>,
    /// When true, the escape-hatch tools in [`REALM_RESTRICTED_TOOLS`] were
    /// withheld from this dispatch and [`Self::call`] denies them explicitly.
    realm_restricted: bool,
}

impl ToolDispatch {
    pub fn new(shared: SharedToolCtx) -> Self {
        let restricted = shared.realm_scope.is_restricted();
        let mut map: HashMap<&'static str, Arc<dyn Tool>> = HashMap::with_capacity(20);
        map.insert("list_databases", tool_list_databases(shared.clone()));
        map.insert("describe_table", tool_describe_table(shared.clone()));
        map.insert("run_sql", tool_run_sql(shared.clone()));
        map.insert("run_kql", tool_run_kql(shared.clone()));
        map.insert("sample_rows", tool_sample_rows(shared.clone()));
        map.insert("explore_schema", tool_explore_schema(shared.clone()));
        map.insert(
            "find_references_to",
            tool_find_references_to(shared.clone()),
        );
        map.insert("graph_traverse", tool_graph_traverse(shared.clone()));
        map.insert("graph_analytics", tool_graph_analytics(shared.clone()));
        // Unified `/v1/search` substrate exposed to agents: hybrid lexical+vector
        // data search + cross-graph node search (the same dispatcher behind
        // POST /v1/search and the Explore UI). Memory mode is intentionally not
        // re-exposed — memory_search/recall_memory already share retrieve() and
        // return a richer payload than the unified envelope.
        map.insert("search", tool_search(shared.clone()));
        map.insert("graph_search", tool_graph_search(shared.clone()));
        // File-contribution + recall (E5): persist a file's structure once, then
        // recall its meaning/relationships cheaply (context-window economy).
        map.insert("contribute_file", tool_contribute_file(shared.clone()));
        map.insert("describe_file", tool_describe_file(shared.clone()));
        map.insert("file_neighbors", tool_file_neighbors(shared.clone()));
        map.insert("recall_file", tool_recall_file(shared.clone()));
        // Agentic Memory tools — let MCP clients (e.g. the Claude Code plugin)
        // search/recall/save durable memories alongside the query tools.
        // `memory_search` is the primary graph-aware hybrid recall entry point.
        map.insert("memory_search", tool_memory_search(shared.clone()));
        map.insert("recall_memory", tool_recall_memory(shared.clone()));
        map.insert("save_memory", tool_save_memory(shared.clone()));
        // Bulk save (one batched embed + commit) and the explicit commit
        // barrier for the async-by-default write path.
        map.insert("save_memories", tool_save_memories(shared.clone()));
        map.insert("flush_memory", tool_flush_memory(shared.clone()));
        map.insert("list_memories", tool_list_memories(shared.clone()));
        map.insert(
            "link_memory_to_entity",
            tool_link_memory_to_entity(shared.clone()),
        );
        // Dynamic ingestion: create virtual resources/entities on the graph,
        // wired to memories + existing catalog (data source) resources.
        map.insert("ingest_entity", tool_ingest_entity(shared.clone()));
        // Curation: re-weight / archive memories during housekeeping.
        map.insert(
            "update_memory_status",
            tool_update_memory_status(shared.clone()),
        );
        map.insert(
            "update_memory_importance",
            tool_update_memory_importance(shared.clone()),
        );
        // Agent-driven conflict resolution.
        map.insert("memory_compare", tool_memory_compare(shared.clone()));
        map.insert("memory_judge", tool_memory_judge(shared.clone()));
        // Structured end-of-session capture.
        map.insert(
            "memory_session_summary",
            tool_memory_session_summary(shared.clone()),
        );
        // Usage-based reinforcement (M8.1): report whether a recalled memory
        // actually helped, and list the reinforcement backstop worklist.
        map.insert("reinforce_memory", tool_reinforce_memory(shared.clone()));
        map.insert("list_memory_usage", tool_list_memory_usage(shared));
        // Realm-restricted callers never see the escape-hatch tools — so
        // `tools/list` doesn't advertise a tool they can't call, and there is
        // no way to reach cross-realm data through raw query/graph/search.
        if restricted {
            map.retain(|name, _| !REALM_RESTRICTED_TOOLS.contains(name));
        }
        Self {
            by_name: Arc::new(map),
            realm_restricted: restricted,
        }
    }

    /// Register the `retrieve_artifact` tool, backed by `store`, so MCP clients
    /// (coding agents) can fetch byte windows of stored log/file artifacts by
    /// `object_path`. Kept off the base [`SharedToolCtx`] so only deployments
    /// with an object store wire it (the server binary; not local-mode tests).
    pub fn with_artifact_store(mut self, store: Arc<dyn object_store::ObjectStore>) -> Self {
        let mut map = (*self.by_name).clone();
        map.insert("retrieve_artifact", tool_retrieve_artifact(store));
        self.by_name = Arc::new(map);
        self
    }

    /// Add the read-only data source tools (`list_data_sources`, `data_source_read`)
    /// so MCP-driven agents — notably Claude CLI dreaming runs — can fill
    /// memory gaps from configured sources. Server mode only (needs the
    /// credential store); local/stdio mode skips this.
    pub fn with_datasource_tools(
        self,
        ctx: pensieve_server::agent::datasource_tools::DataSourceToolCtx,
    ) -> Self {
        let mut map: HashMap<&'static str, Arc<dyn Tool>> = (*self.by_name).clone();
        map.insert(
            "list_data_sources",
            pensieve_server::agent::datasource_tools::tool_list_data_sources(ctx.clone()),
        );
        map.insert(
            "data_source_read",
            pensieve_server::agent::datasource_tools::tool_data_source_read(ctx),
        );
        Self {
            by_name: Arc::new(map),
            realm_restricted: self.realm_restricted,
        }
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
        // Realm-scoped tokens: name the policy instead of a bare "unknown tool"
        // when they reach for a withheld escape-hatch tool.
        if self.realm_restricted && REALM_RESTRICTED_TOOLS.contains(&name) {
            return Err(ErrorObject::new(
                ErrorCode::InvalidParams as i64,
                format!("tool `{name}` is not available to realm-scoped tokens"),
            ));
        }
        let Some(tool) = self.by_name.get(name).cloned() else {
            return Err(ErrorObject::new(
                ErrorCode::MethodNotFound as i64,
                format!("unknown tool: {name}"),
            ));
        };
        let ctx = Arc::new(SimpleToolContext::new("pensieve-mcp"));
        // Span names must be static in `tracing`; otel.name carries the
        // per-tool display name through to the exported span.
        let span = tracing::info_span!(
            target: "pensieve_telemetry",
            "tool.call",
            otel.name = %format!("tool.{name}"),
            tool.name = %name,
        );
        match tracing::Instrument::instrument(tool.execute(ctx, arguments), span).await {
            Ok(value) => Ok(json!({
                "content": [
                    {"type": "text", "text": serde_json::to_string(&value).expect("serializing serde_json::Value to string is infallible")}
                ],
                "isError": false,
                "structuredContent": value,
            })),
            Err(e) => Err(ErrorObject::new(
                ErrorCode::InternalError as i64,
                format!("tool {name}: {e}"),
            )),
        }
    }
}

/// The ingredients needed to rebuild a [`ToolDispatch`] per request with a
/// caller-specific [`RealmScope`](pensieve_server::auth::RealmScope). Held in
/// `McpState` so the server-mode MCP transport can serve realm-restricted
/// tokens a scoped tool set without disturbing the startup-built fast path used
/// by the (overwhelming majority) unrestricted callers.
///
/// `None` in local/stdio mode: there is no per-request `Principal` there, so
/// no scoped dispatch is ever needed.
#[derive(Clone)]
pub struct DispatchBuilder {
    pub shared: SharedToolCtx,
    pub artifact_store: Option<Arc<dyn object_store::ObjectStore>>,
    pub datasource_ctx: Option<pensieve_server::agent::datasource_tools::DataSourceToolCtx>,
}

impl DispatchBuilder {
    /// Build a dispatch for a caller with the given realm scope. For an
    /// unrestricted scope this reproduces the full tool set (including the
    /// artifact/datasource extensions); for a restricted scope the escape-hatch
    /// tools are withheld and the extensions are skipped entirely.
    pub fn build(&self, scope: pensieve_server::auth::RealmScope) -> ToolDispatch {
        let mut shared = self.shared.clone();
        shared.realm_scope = scope.clone();
        let mut d = ToolDispatch::new(shared);
        if !scope.is_restricted() {
            if let Some(s) = &self.artifact_store {
                d = d.with_artifact_store(s.clone());
            }
            if let Some(c) = &self.datasource_ctx {
                d = d.with_datasource_tools(c.clone());
            }
        }
        d
    }
}
