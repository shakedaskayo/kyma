# Graph Layer G1b.1 — Catalog Graph Registration + CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make a property-graph a first-class, catalog-registered object: a `graph_registrations` table, a `GraphRegistration` type + `Catalog` CRUD methods, and `pensieve-cli create-graph / list-graphs / drop-graph`. This is what `StoredGraphProvider` (G1b.2) reads to serve `/v1/graph/{name}/*` over registered node/edge tables, and what the self-learning feature later writes into.

**Architecture:** Mirror the existing **Dashboard CRUD** exactly (the closest analog). A graph registration binds a `database`, a `name`, a `node_table` + `edge_table`, and the column roles (`id/label/src/dst/type`, optional `realm`). One `impl Catalog` exists (`PostgresCatalog`); tests drive it via testcontainers through `pensieve-server::test_support`.

**Tech Stack:** Rust, `sqlx` (Postgres), `clap` (CLI), `uuid`, `chrono`. Migration via `sqlx::migrate!("./migrations")`.

**Reference (exact templates):** `crates/pensieve-core/src/catalog.rs` (Dashboard struct + trait methods at ~207-216, ~539-610), `crates/pensieve-catalog/src/lib.rs` (PostgresCatalog dashboard impls ~917-998 + `row_to_dashboard`/`sql_err` ~1212-1227 + `sqlx::migrate!` at ~59), `crates/pensieve-catalog/migrations/006_dashboards.sql`, `crates/pensieve-core/src/errors.rs` (`CatalogError` ~44-67), `crates/pensieve-cli/src/main.rs` (clap enum + `connect()` ~148 + `parse_schema_spec`).

**Working dir:** worktree `/Users/shakedaskayo/shaked/projects/pensieve/.claude/worktrees/feature+graph-layer`. Tests need Docker (testcontainers). Scope builds: `cargo build -p pensieve-core -p pensieve-catalog -p pensieve-cli`.

**Reserved name:** `schema` is reserved for the synthetic schema-graph — the CLI and `create_graph` must reject a registration named `schema` (G1b.2's router resolves `schema` to `SchemaGraphProvider`).

---

## File structure
- `crates/pensieve-core/src/catalog.rs` — add `GraphRegistration` + `GraphSpec` structs + 4 `Catalog` trait methods (each with the default→`_in_tenant` delegation).
- `crates/pensieve-core/src/errors.rs` — add `CatalogError::GraphNotFound`.
- `crates/pensieve-catalog/migrations/008_graphs.sql` — new table.
- `crates/pensieve-catalog/src/lib.rs` — `PostgresCatalog` impl of the 4 `_in_tenant` methods + `row_to_graph` helper.
- `crates/pensieve-server/src/graph_handler.rs` (test module) OR a new `crates/pensieve-catalog/tests/graphs.rs` — registration roundtrip test (use whichever testcontainers harness is simplest; this plan uses `pensieve-server::test_support`).
- `crates/pensieve-cli/src/main.rs` — `CreateGraph` / `ListGraphs` / `DropGraph` subcommands.

---

## Task 1: types + trait methods + error variant (pensieve-core)

**Files:** `crates/pensieve-core/src/catalog.rs`, `crates/pensieve-core/src/errors.rs`.

- [ ] **Step 1: add the error variant** in `crates/pensieve-core/src/errors.rs`, in the `CatalogError` enum (after `DashboardNotFound`):
```rust
    #[error("graph registration not found: {database}.{name}")]
    GraphNotFound { database: String, name: String },
```

- [ ] **Step 2: add the structs** in `crates/pensieve-core/src/catalog.rs` (near the `Dashboard` struct, e.g. just before the `// -------------------- Dashboards` section or in a new `// -------------------- Graphs` section):
```rust
/// A registered property-graph: binds a node table + edge table in a database,
/// with the column roles that identify nodes/edges. Read by the graph layer's
/// stored-graph provider to serve `/v1/graph/<name>/*`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphRegistration {
    pub id: uuid::Uuid,
    pub database: String,
    pub name: String,
    pub node_table: String,
    pub edge_table: String,
    pub id_col: String,
    pub label_col: String,
    pub src_col: String,
    pub dst_col: String,
    pub type_col: String,
    pub realm_col: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Column-role spec supplied when registering a graph. Defaults match the
/// conventional node/edge schema (see the graph-layer design §3.1).
#[derive(Debug, Clone)]
pub struct GraphSpec {
    pub node_table: String,
    pub edge_table: String,
    pub id_col: String,
    pub label_col: String,
    pub src_col: String,
    pub dst_col: String,
    pub type_col: String,
    pub realm_col: Option<String>,
}

impl GraphSpec {
    /// Conventional defaults: nodes(`id`,`labels`), edges(`src`,`dst`,`type`),
    /// optional `realm`. Caller overrides the column names as needed.
    pub fn with_defaults(node_table: impl Into<String>, edge_table: impl Into<String>) -> Self {
        Self {
            node_table: node_table.into(),
            edge_table: edge_table.into(),
            id_col: "id".into(),
            label_col: "labels".into(),
            src_col: "src".into(),
            dst_col: "dst".into(),
            type_col: "type".into(),
            realm_col: None,
        }
    }
}
```

- [ ] **Step 3: add the trait methods** to the `Catalog` trait in `catalog.rs` (mirror the dashboard default→`_in_tenant` pattern; place in the new Graphs section):
```rust
    // -------------------- Graphs --------------------

    /// Register a property-graph in `database` under `name`.
    async fn create_graph(
        &self,
        database: &str,
        name: &str,
        spec: GraphSpec,
    ) -> Result<GraphRegistration, CatalogError> {
        self.create_graph_in_tenant(crate::tenant::DEFAULT_TENANT, database, name, spec).await
    }
    async fn create_graph_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        name: &str,
        spec: GraphSpec,
    ) -> Result<GraphRegistration, CatalogError>;

    /// List all graphs registered in `database`.
    async fn list_graphs(&self, database: &str) -> Result<Vec<GraphRegistration>, CatalogError> {
        self.list_graphs_in_tenant(crate::tenant::DEFAULT_TENANT, database).await
    }
    async fn list_graphs_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
    ) -> Result<Vec<GraphRegistration>, CatalogError>;

    /// Look up a single graph by `database` + `name`.
    async fn get_graph(
        &self,
        database: &str,
        name: &str,
    ) -> Result<Option<GraphRegistration>, CatalogError> {
        self.get_graph_in_tenant(crate::tenant::DEFAULT_TENANT, database, name).await
    }
    async fn get_graph_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        name: &str,
    ) -> Result<Option<GraphRegistration>, CatalogError>;

    /// Drop a graph registration (does NOT drop the underlying tables).
    /// Returns true if a registration was removed.
    async fn drop_graph(&self, database: &str, name: &str) -> Result<bool, CatalogError> {
        self.drop_graph_in_tenant(crate::tenant::DEFAULT_TENANT, database, name).await
    }
    async fn drop_graph_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        name: &str,
    ) -> Result<bool, CatalogError>;
```

- [ ] **Step 4:** `cargo build -p pensieve-core` → it will FAIL to compile `pensieve-catalog` later (PostgresCatalog doesn't yet implement the new methods), but `pensieve-core` itself compiles. Run `cargo build -p pensieve-core` → clean. (pensieve-catalog impl is Task 2.)

- [ ] **Step 5: Commit:**
```bash
git add crates/pensieve-core/src/catalog.rs crates/pensieve-core/src/errors.rs
git commit -m "feat(catalog): GraphRegistration type + Catalog graph CRUD trait methods"
```

---

## Task 2: migration + PostgresCatalog implementation

**Files:** `crates/pensieve-catalog/migrations/008_graphs.sql`, `crates/pensieve-catalog/src/lib.rs`.

- [ ] **Step 1: migration** — READ `crates/pensieve-catalog/migrations/006_dashboards.sql` to match the exact `id` default + style, then create `crates/pensieve-catalog/migrations/008_graphs.sql`:
```sql
-- Graph registrations: bind a node table + edge table (with column roles)
-- in a database to a named property-graph. Tables themselves are ordinary
-- pensieve tables; this is metadata only.
CREATE TABLE graph_registrations (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   uuid NOT NULL,
    database    text NOT NULL,
    name        text NOT NULL,
    node_table  text NOT NULL,
    edge_table  text NOT NULL,
    id_col      text NOT NULL,
    label_col   text NOT NULL,
    src_col     text NOT NULL,
    dst_col     text NOT NULL,
    type_col    text NOT NULL,
    realm_col   text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, database, name)
);
CREATE INDEX graph_registrations_tenant_idx ON graph_registrations (tenant_id);
```
(If `006_dashboards.sql` uses a different uuid-default idiom than `gen_random_uuid()`, match it. pg16 has `gen_random_uuid()` built in.)

- [ ] **Step 2: row mapper** — add near `row_to_dashboard` in `crates/pensieve-catalog/src/lib.rs`:
```rust
fn row_to_graph(row: &sqlx::postgres::PgRow) -> std::result::Result<pensieve_core::catalog::GraphRegistration, CatalogError> {
    use sqlx::Row as _;
    Ok(pensieve_core::catalog::GraphRegistration {
        id: row.try_get("id").map_err(sql_err)?,
        database: row.try_get("database").map_err(sql_err)?,
        name: row.try_get("name").map_err(sql_err)?,
        node_table: row.try_get("node_table").map_err(sql_err)?,
        edge_table: row.try_get("edge_table").map_err(sql_err)?,
        id_col: row.try_get("id_col").map_err(sql_err)?,
        label_col: row.try_get("label_col").map_err(sql_err)?,
        src_col: row.try_get("src_col").map_err(sql_err)?,
        dst_col: row.try_get("dst_col").map_err(sql_err)?,
        type_col: row.try_get("type_col").map_err(sql_err)?,
        realm_col: row.try_get("realm_col").map_err(sql_err)?,
        created_at: row.try_get("created_at").map_err(sql_err)?,
        updated_at: row.try_get("updated_at").map_err(sql_err)?,
    })
}
```
(Confirm the exact `use` path for `GraphRegistration`/`GraphSpec`/`TenantId` matches how other types are referenced in this file — e.g. the file likely `use pensieve_core::catalog::{...}` at the top; add `GraphRegistration, GraphSpec` there and use the short names.)

- [ ] **Step 3: impl the 4 methods** inside `impl Catalog for PostgresCatalog` (mirror the dashboard impls). Use a stable column list `id, database, name, node_table, edge_table, id_col, label_col, src_col, dst_col, type_col, realm_col, created_at, updated_at`:
```rust
    async fn create_graph_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        name: &str,
        spec: GraphSpec,
    ) -> std::result::Result<GraphRegistration, CatalogError> {
        let row = sqlx::query(
            "INSERT INTO graph_registrations
               (tenant_id, database, name, node_table, edge_table,
                id_col, label_col, src_col, dst_col, type_col, realm_col)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             RETURNING id, database, name, node_table, edge_table,
                       id_col, label_col, src_col, dst_col, type_col, realm_col,
                       created_at, updated_at",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .bind(name)
        .bind(&spec.node_table)
        .bind(&spec.edge_table)
        .bind(&spec.id_col)
        .bind(&spec.label_col)
        .bind(&spec.src_col)
        .bind(&spec.dst_col)
        .bind(&spec.type_col)
        .bind(spec.realm_col.as_deref())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        row_to_graph(&row)
    }

    async fn list_graphs_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
    ) -> std::result::Result<Vec<GraphRegistration>, CatalogError> {
        let rows = sqlx::query(
            "SELECT id, database, name, node_table, edge_table,
                    id_col, label_col, src_col, dst_col, type_col, realm_col,
                    created_at, updated_at
             FROM graph_registrations
             WHERE tenant_id = $1 AND database = $2
             ORDER BY name",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        rows.iter().map(row_to_graph).collect()
    }

    async fn get_graph_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        name: &str,
    ) -> std::result::Result<Option<GraphRegistration>, CatalogError> {
        let maybe = sqlx::query(
            "SELECT id, database, name, node_table, edge_table,
                    id_col, label_col, src_col, dst_col, type_col, realm_col,
                    created_at, updated_at
             FROM graph_registrations
             WHERE tenant_id = $1 AND database = $2 AND name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        match maybe {
            Some(row) => Ok(Some(row_to_graph(&row)?)),
            None => Ok(None),
        }
    }

    async fn drop_graph_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        name: &str,
    ) -> std::result::Result<bool, CatalogError> {
        let res = sqlx::query(
            "DELETE FROM graph_registrations WHERE tenant_id = $1 AND database = $2 AND name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .bind(name)
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }
```
Add `GraphRegistration, GraphSpec` to the `pensieve_core::catalog` import at the top of the file (alongside `Dashboard`, etc.).

- [ ] **Step 4:** `cargo build -p pensieve-catalog` → clean.

- [ ] **Step 5: Commit:**
```bash
git add crates/pensieve-catalog/migrations/008_graphs.sql crates/pensieve-catalog/src/lib.rs
git commit -m "feat(catalog): graph_registrations table + PostgresCatalog graph CRUD"
```

---

## Task 3: registration roundtrip integration test

**Files:** `crates/pensieve-server/src/graph_handler.rs` (extend the `tests` module).

- [ ] **Step 1: write the test** — append to the `tests` module (gated `#[cfg(all(test, feature = "test-support"))]`):
```rust
    #[tokio::test]
    async fn graph_registration_crud_roundtrip() {
        use pensieve_core::catalog::GraphSpec;
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let cat = &state.catalog;

        // none registered yet
        assert!(cat.list_graphs("obs").await.unwrap().is_empty());
        assert!(cat.get_graph("obs", "kg").await.unwrap().is_none());

        // create
        let mut spec = GraphSpec::with_defaults("kg_nodes", "kg_edges");
        spec.realm_col = Some("realm".into());
        let reg = cat.create_graph("obs", "kg", spec).await.unwrap();
        assert_eq!(reg.name, "kg");
        assert_eq!(reg.node_table, "kg_nodes");
        assert_eq!(reg.edge_table, "kg_edges");
        assert_eq!(reg.id_col, "id");
        assert_eq!(reg.realm_col.as_deref(), Some("realm"));

        // get + list
        let got = cat.get_graph("obs", "kg").await.unwrap().unwrap();
        assert_eq!(got.id, reg.id);
        assert_eq!(cat.list_graphs("obs").await.unwrap().len(), 1);

        // drop
        assert!(cat.drop_graph("obs", "kg").await.unwrap());
        assert!(!cat.drop_graph("obs", "kg").await.unwrap()); // idempotent: now false
        assert!(cat.get_graph("obs", "kg").await.unwrap().is_none());
    }
```

- [ ] **Step 2:** `cargo test -p pensieve-server --features test-support graph_handler::tests::graph_registration_crud_roundtrip` → PASS (needs Docker).

- [ ] **Step 3: Commit:**
```bash
git add crates/pensieve-server/src/graph_handler.rs
git commit -m "test(catalog): graph registration CRUD roundtrip"
```

---

## Task 4: pensieve-cli `create-graph` / `list-graphs` / `drop-graph`

**Files:** `crates/pensieve-cli/src/main.rs`.

- [ ] **Step 1: add the subcommands** to the `Command` enum:
```rust
    /// Register a property-graph (binds a node table + edge table).
    CreateGraph {
        #[arg(long)] db: String,
        #[arg(long)] name: String,
        #[arg(long)] nodes: String,                 // node table
        #[arg(long)] edges: String,                 // edge table
        #[arg(long, default_value = "id")] id_col: String,
        #[arg(long, default_value = "labels")] label_col: String,
        #[arg(long, default_value = "src")] src_col: String,
        #[arg(long, default_value = "dst")] dst_col: String,
        #[arg(long, default_value = "type")] type_col: String,
        #[arg(long)] realm_col: Option<String>,
    },
    /// List registered graphs in a database.
    ListGraphs { #[arg(long)] db: String },
    /// Drop a graph registration (leaves the underlying tables intact).
    DropGraph { #[arg(long)] db: String, #[arg(long)] name: String },
```

- [ ] **Step 2: handle them** in the `match command` block (mirroring how `CreateTable`/`ListTables` connect + call the catalog). Reject the reserved name `schema`:
```rust
        Command::CreateGraph {
            db, name, nodes, edges, id_col, label_col, src_col, dst_col, type_col, realm_col,
        } => {
            if name == "schema" {
                anyhow::bail!("'schema' is reserved for the synthetic schema-graph; choose another name");
            }
            let cat = connect(&cli.catalog_url).await?;
            let spec = pensieve_core::catalog::GraphSpec {
                node_table: nodes,
                edge_table: edges,
                id_col,
                label_col,
                src_col,
                dst_col,
                type_col,
                realm_col,
            };
            let reg = cat.create_graph(&db, &name, spec).await?;
            println!("registered graph '{}' in db '{}' (nodes={}, edges={})", reg.name, db, reg.node_table, reg.edge_table);
        }
        Command::ListGraphs { db } => {
            let cat = connect(&cli.catalog_url).await?;
            let graphs = cat.list_graphs(&db).await?;
            if graphs.is_empty() {
                println!("(no graphs registered in '{db}')");
            } else {
                for g in graphs {
                    println!("{}\tnodes={}\tedges={}\trealm={}", g.name, g.node_table, g.edge_table, g.realm_col.as_deref().unwrap_or("-"));
                }
            }
        }
        Command::DropGraph { db, name } => {
            let cat = connect(&cli.catalog_url).await?;
            if cat.drop_graph(&db, &name).await? {
                println!("dropped graph '{name}' from '{db}'");
            } else {
                println!("no graph '{name}' in '{db}'");
            }
        }
```
(Match the surrounding code's exact style — whether arms use `connect(&cli.catalog_url)` or a pre-built catalog; confirm `pensieve_core` is a dependency of pensieve-cli, it is.)

- [ ] **Step 3:** `cargo build -p pensieve-cli` → clean. Sanity: `./target/debug/pensieve-cli create-graph --help` shows the flags (no DB needed for --help).

- [ ] **Step 4: Commit:**
```bash
git add crates/pensieve-cli/src/main.rs
git commit -m "feat(cli): create-graph / list-graphs / drop-graph"
```

---

## Task 5: verify
- [ ] `cargo build -p pensieve-core -p pensieve-catalog -p pensieve-cli -p pensieve-server` → clean.
- [ ] `cargo test -p pensieve-server --features test-support graph_handler::tests::graph_registration_crud_roundtrip` → PASS.
- [ ] `cargo clippy -p pensieve-catalog -p pensieve-cli 2>&1 | tail -20` → no new warnings from the changed code.

---

## Self-review notes
- **Pattern fidelity:** the trait methods follow the dashboard default→`_in_tenant` delegation; only `PostgresCatalog` implements the `_in_tenant` variants (no mock catalog exists).
- **`drop_graph` is idempotent** (returns `false` if nothing matched) — the test asserts the second drop returns false.
- **Reserved name `schema`** rejected at the CLI; G1b.2's server router resolves `schema`→`SchemaGraphProvider` and registered names→`StoredGraphProvider`.
- **Out of scope (G1b.2):** `StoredGraphProvider` (querying node/edge tables → GraphNode/GraphRelationship), the server routing that dispatches `/v1/graph/{name}` to stored vs schema providers, and listing registered graphs in `GET /v1/graph`. This plan only persists + manages the registration metadata.
- **Migration safety:** `008_graphs.sql` CREATEs a fresh table (no backfill), so it applies cleanly on existing catalogs. It honors the v1 "no extent-format change" rule (this is catalog metadata, not extent data).
