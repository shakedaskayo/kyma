# Discover — Engine Saved Views CRUD Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Ship the saved-views surface for Discover: a single new table `saved_views`, four CRUD endpoints under `/v1/explore/views`, and the `SavedViewLookup` wiring so the search handler can resolve `scope.kind: "view"`.

**Architecture:** One additive migration (013), one catalog module (`saved_views.rs`) implementing CRUD against Postgres, one Axum handler module with four routes mounted under the existing **write** router for create/patch/delete and the **read** router for list. The `SavedViewLookup` trait introduced by Plan A is implemented by a thin adapter wrapping the catalog method.

**Tech Stack:** Rust, Axum, sqlx (already in use by `pensieve-catalog`), serde, uuid.

**Reference spec:** `docs/superpowers/specs/2026-05-28-explore-discover-refactor-design.md` (section 4.2).

**Prereqs:** Plan A `2026-05-28-discover-engine-search.md` must be merged (this plan adds wiring that targets the handler shipped there).

---

## File Structure

| File                                                       | Action  | Responsibility                              |
|------------------------------------------------------------|---------|---------------------------------------------|
| `crates/pensieve-catalog/migrations/013_saved_views.sql`       | Create  | Postgres table + indexes                    |
| `crates/pensieve-catalog/src/saved_views.rs`                   | Create  | CRUD methods against the table              |
| `crates/pensieve-catalog/src/lib.rs`                           | Modify  | `pub mod saved_views;` export               |
| `crates/pensieve-server/src/discover/saved_views_handler.rs`   | Create  | Axum CRUD handlers + types                  |
| `crates/pensieve-server/src/discover/saved_views_lookup.rs`    | Create  | `SavedViewLookup` adapter wiring the catalog into Plan A's resolver |
| `crates/pensieve-server/src/discover/mod.rs`                   | Modify  | `pub mod saved_views_handler; pub mod saved_views_lookup;` |
| `crates/pensieve-server/src/discover/handler.rs`               | Modify  | Pass `Some(&lookup)` into `resolve_scope` (replaces the `None`) |
| `crates/pensieve-server/src/lib.rs`                            | Modify  | Mount `GET /v1/explore/views` (read router) + `POST/PATCH/DELETE` (write router) |
| `crates/pensieve-server/tests/discover_saved_views_it.rs`      | Create  | Integration test: create → list → use as scope → delete |

---

## Task 1: Migration

**Files:** `crates/pensieve-catalog/migrations/013_saved_views.sql`

- [ ] **Step 1: Write the migration**

```sql
-- 013_saved_views.sql
-- Saved Discover "views": a named scope (set of db.table patterns) owned by a user.
-- Scope is the only thing persisted; search text, filters, and time range stay
-- ephemeral (per spec section 4.2).

CREATE TABLE IF NOT EXISTS saved_views (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID        NOT NULL,
    owner_subject   TEXT        NOT NULL,
    name            TEXT        NOT NULL,
    sources_json    JSONB       NOT NULL,
    columns_json    JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT saved_views_owner_name_unique
        UNIQUE (tenant_id, owner_subject, name)
);

CREATE INDEX IF NOT EXISTS saved_views_by_owner
    ON saved_views (tenant_id, owner_subject, updated_at DESC);
```

- [ ] **Step 2: Confirm `pgcrypto` (for `gen_random_uuid`) is available**

```bash
grep -rn 'CREATE EXTENSION' crates/pensieve-catalog/migrations/
```

If no migration enables `pgcrypto`, prepend to `013_saved_views.sql`:

```sql
CREATE EXTENSION IF NOT EXISTS pgcrypto;
```

- [ ] **Step 3: Run the engine and watch migration apply**

```bash
cargo run -p pensieve-bin 2>&1 | grep -i 'migration\|saved_views' | head
```

Expected: a log line indicating `013_saved_views.sql` was applied. Stop the engine.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-catalog/migrations/013_saved_views.sql
git commit -m "feat(catalog): migration 013 — saved_views table"
```

---

## Task 2: Catalog CRUD methods

**Files:**
- Create: `crates/pensieve-catalog/src/saved_views.rs`
- Modify: `crates/pensieve-catalog/src/lib.rs`

- [ ] **Step 1: Add module export**

In `crates/pensieve-catalog/src/lib.rs`, near the other `pub mod` lines, add:

```rust
pub mod saved_views;
```

- [ ] **Step 2: Write the CRUD module with failing tests**

Write `crates/pensieve-catalog/src/saved_views.rs`:

```rust
//! CRUD against `saved_views`. All methods scope on (tenant_id, owner_subject).

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedView {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub owner_subject: String,
    pub name: String,
    pub sources: Vec<String>,
    pub columns: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NewSavedView {
    pub name: String,
    pub sources: Vec<String>,
    pub columns: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateSavedView {
    pub name: Option<String>,
    pub sources: Option<Vec<String>>,
    pub columns: Option<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum SavedViewError {
    #[error("saved view not found")]
    NotFound,
    #[error("name conflict")]
    NameConflict,
    #[error("database error: {0}")]
    Db(String),
}

pub async fn create(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
    input: NewSavedView,
) -> Result<SavedView, SavedViewError> {
    let sources = serde_json::to_value(&input.sources).expect("json infallible");
    let rec = sqlx::query!(
        r#"
        INSERT INTO saved_views (tenant_id, owner_subject, name, sources_json, columns_json)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, created_at, updated_at
        "#,
        tenant_id, owner, input.name, sources, input.columns,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| {
        if let Some(db_err) = e.as_database_error() {
            if db_err.code().as_deref() == Some("23505") {
                return SavedViewError::NameConflict;
            }
        }
        SavedViewError::Db(e.to_string())
    })?;

    Ok(SavedView {
        id: rec.id,
        tenant_id,
        owner_subject: owner.to_string(),
        name: input.name,
        sources: input.sources,
        columns: input.columns,
        created_at: rec.created_at,
        updated_at: rec.updated_at,
    })
}

pub async fn list(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
) -> Result<Vec<SavedView>, SavedViewError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, name, sources_json, columns_json, created_at, updated_at
        FROM saved_views
        WHERE tenant_id = $1 AND owner_subject = $2
        ORDER BY updated_at DESC
        "#,
        tenant_id, owner,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| SavedViewError::Db(e.to_string()))?;

    rows.into_iter()
        .map(|r| {
            let sources: Vec<String> = serde_json::from_value(r.sources_json)
                .map_err(|e| SavedViewError::Db(format!("decode sources: {e}")))?;
            Ok(SavedView {
                id: r.id,
                tenant_id,
                owner_subject: owner.to_string(),
                name: r.name,
                sources,
                columns: r.columns_json,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
        })
        .collect()
}

pub async fn get(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
    id: Uuid,
) -> Result<SavedView, SavedViewError> {
    let r = sqlx::query!(
        r#"
        SELECT id, name, sources_json, columns_json, created_at, updated_at
        FROM saved_views
        WHERE tenant_id = $1 AND owner_subject = $2 AND id = $3
        "#,
        tenant_id, owner, id,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| SavedViewError::Db(e.to_string()))?
    .ok_or(SavedViewError::NotFound)?;

    let sources: Vec<String> = serde_json::from_value(r.sources_json)
        .map_err(|e| SavedViewError::Db(format!("decode sources: {e}")))?;
    Ok(SavedView {
        id: r.id, tenant_id, owner_subject: owner.to_string(),
        name: r.name, sources, columns: r.columns_json,
        created_at: r.created_at, updated_at: r.updated_at,
    })
}

pub async fn update(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
    id: Uuid,
    input: UpdateSavedView,
) -> Result<SavedView, SavedViewError> {
    let mut tx = pool.begin().await.map_err(|e| SavedViewError::Db(e.to_string()))?;
    let current = get(pool, tenant_id, owner, id).await?;

    let new_name = input.name.unwrap_or(current.name.clone());
    let new_sources = input.sources.unwrap_or(current.sources.clone());
    let new_columns = input.columns.or(current.columns.clone());
    let sources_json = serde_json::to_value(&new_sources).expect("json infallible");

    sqlx::query!(
        r#"
        UPDATE saved_views
        SET name = $1, sources_json = $2, columns_json = $3, updated_at = NOW()
        WHERE tenant_id = $4 AND owner_subject = $5 AND id = $6
        "#,
        new_name, sources_json, new_columns, tenant_id, owner, id,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if let Some(db_err) = e.as_database_error() {
            if db_err.code().as_deref() == Some("23505") {
                return SavedViewError::NameConflict;
            }
        }
        SavedViewError::Db(e.to_string())
    })?;
    tx.commit().await.map_err(|e| SavedViewError::Db(e.to_string()))?;

    get(pool, tenant_id, owner, id).await
}

pub async fn delete(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
    id: Uuid,
) -> Result<(), SavedViewError> {
    let res = sqlx::query!(
        "DELETE FROM saved_views WHERE tenant_id=$1 AND owner_subject=$2 AND id=$3",
        tenant_id, owner, id,
    )
    .execute(pool)
    .await
    .map_err(|e| SavedViewError::Db(e.to_string()))?;
    if res.rows_affected() == 0 {
        return Err(SavedViewError::NotFound);
    }
    Ok(())
}
```

- [ ] **Step 3: Compile-check**

```bash
cargo check -p pensieve-catalog
```

Expected: compiles. If `chrono` or `uuid` aren't in scope, add `chrono = { workspace = true, features = ["serde"] }` and `uuid = { workspace = true }` to `crates/pensieve-catalog/Cargo.toml`.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-catalog/src/saved_views.rs crates/pensieve-catalog/src/lib.rs crates/pensieve-catalog/Cargo.toml
git commit -m "feat(catalog): saved_views CRUD module"
```

---

## Task 3: HTTP handlers

**Files:**
- Create: `crates/pensieve-server/src/discover/saved_views_handler.rs`
- Modify: `crates/pensieve-server/src/discover/mod.rs`

- [ ] **Step 1: Add module to `discover/mod.rs`**

```rust
pub mod saved_views_handler;
pub mod saved_views_lookup;
```

- [ ] **Step 2: Write the handler module**

Write `crates/pensieve-server/src/discover/saved_views_handler.rs`:

```rust
//! CRUD endpoints for saved Discover views.
//!
//! Routes (registered in `pensieve_server::lib`):
//!   GET    /v1/explore/views        → list (owner-scoped)
//!   POST   /v1/explore/views        → create
//!   PATCH  /v1/explore/views/:id    → update
//!   DELETE /v1/explore/views/:id    → delete

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use pensieve_catalog::saved_views::{
    create as cat_create, delete as cat_delete, list as cat_list, update as cat_update,
    NewSavedView, SavedView, SavedViewError, UpdateSavedView,
};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::principal::{Principal, TenantId};

#[derive(Clone)]
pub struct SavedViewsState {
    pub pool: Arc<PgPool>,
}

pub async fn list_views(
    State(state): State<SavedViewsState>,
    Extension(tenant): Extension<TenantId>,
    Extension(principal): Extension<Principal>,
) -> Response {
    match cat_list(&state.pool, tenant.0, &principal.subject).await {
        Ok(views) => Json(views).into_response(),
        Err(e) => map_error(e),
    }
}

pub async fn create_view(
    State(state): State<SavedViewsState>,
    Extension(tenant): Extension<TenantId>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<NewSavedView>,
) -> Response {
    if body.name.trim().is_empty() {
        return error("bad_request", "name is required", StatusCode::BAD_REQUEST);
    }
    if body.sources.is_empty() {
        return error("bad_request", "sources must be non-empty", StatusCode::BAD_REQUEST);
    }
    match cat_create(&state.pool, tenant.0, &principal.subject, body).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => map_error(e),
    }
}

pub async fn update_view(
    State(state): State<SavedViewsState>,
    Extension(tenant): Extension<TenantId>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateSavedView>,
) -> Response {
    match cat_update(&state.pool, tenant.0, &principal.subject, id, body).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => map_error(e),
    }
}

pub async fn delete_view(
    State(state): State<SavedViewsState>,
    Extension(tenant): Extension<TenantId>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Response {
    match cat_delete(&state.pool, tenant.0, &principal.subject, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_error(e),
    }
}

fn map_error(e: SavedViewError) -> Response {
    match e {
        SavedViewError::NotFound => error("not_found", "saved view not found", StatusCode::NOT_FOUND),
        SavedViewError::NameConflict => error("name_conflict", "a saved view with this name already exists", StatusCode::CONFLICT),
        SavedViewError::Db(msg) => error("db_error", &msg, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

fn error(code: &str, msg: &str, status: StatusCode) -> Response {
    (status, Json(serde_json::json!({"error":{"code":code,"message":msg}}))).into_response()
}
```

- [ ] **Step 3: Compile-check**

```bash
cargo check -p pensieve-server
```

If `crate::auth::principal::{Principal, TenantId}` paths differ, run `grep -rn 'struct Principal\|struct TenantId' crates/pensieve-server/src/auth/` and adjust the imports. The handler logic is unchanged.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/discover/saved_views_handler.rs crates/pensieve-server/src/discover/mod.rs
git commit -m "feat(discover): saved_views CRUD handlers"
```

---

## Task 4: `SavedViewLookup` adapter

**Files:**
- Create: `crates/pensieve-server/src/discover/saved_views_lookup.rs`

- [ ] **Step 1: Write the adapter**

```rust
//! Adapter that implements the `SavedViewLookup` trait introduced by Plan A
//! using the catalog `saved_views` module.

use std::sync::Arc;
use sqlx::PgPool;
use uuid::Uuid;

use super::scope::SavedViewLookup;
use pensieve_catalog::saved_views;

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
```

- [ ] **Step 2: Wire it into the search handler**

Edit `crates/pensieve-server/src/discover/handler.rs` — locate the call:

```rust
let resolved = match resolve_scope(&payload.scope, state.catalog.clone(), None, max_sources).await {
```

and replace `None` with a lookup constructed from the per-request `Extension<TenantId>` + `Extension<Principal>` and the pool. To get the pool into the handler, extend `QueryState` (or introduce a sibling state like `DiscoverState`) to carry `Arc<PgPool>`.

Minimal patch (in `handler.rs`):

```rust
use crate::auth::principal::{Principal, TenantId};
use axum::Extension;

pub async fn discover_search_handler(
    State(state): State<QueryState>,
    Extension(tenant): Extension<TenantId>,
    Extension(principal): Extension<Principal>,
    req: Request<Body>,
) -> Response {
    // ... existing parse logic ...

    let lookup = crate::discover::saved_views_lookup::CatalogSavedViewLookup {
        pool: state.pg_pool.clone(),
        tenant_id: tenant.0,
        owner_subject: principal.subject.clone(),
    };

    let resolved = match resolve_scope(&payload.scope, state.catalog.clone(), Some(&lookup), max_sources).await {
        // ...
    };
    // ... rest unchanged ...
}
```

If `QueryState` does not already expose `pg_pool`, add a field:

```rust
pub struct QueryState {
    // existing fields...
    pub pg_pool: Arc<sqlx::PgPool>,
}
```

and thread it through the constructor in `pensieve-bin/src/main.rs` (search for where `QueryState { catalog, format, node_id }` is built).

- [ ] **Step 3: Compile-check**

```bash
cargo check -p pensieve-server -p pensieve-bin
```

Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/discover/saved_views_lookup.rs \
        crates/pensieve-server/src/discover/handler.rs \
        crates/pensieve-server/src/lib.rs \
        crates/pensieve-bin/src/main.rs
git commit -m "feat(discover): wire SavedViewLookup into search handler"
```

---

## Task 5: Mount routes

**Files:**
- Modify: `crates/pensieve-server/src/lib.rs`

- [ ] **Step 1: Add the GET route to the read router**

In `pub fn router(state: QueryState)`, after the existing `/v1/explore/search` route, add:

```rust
use discover::saved_views_handler::{list_views, SavedViewsState};
let views_read_state = SavedViewsState { pool: state.pg_pool.clone() };
let views_read_router = Router::new()
    .route("/v1/explore/views", get(list_views))
    .with_state(views_read_state);
```

And `.merge(views_read_router)` into the returned router.

- [ ] **Step 2: Add a write router**

Below `pub fn dashboards_write_router(...)`, add:

```rust
pub fn discover_views_write_router(pool: Arc<sqlx::PgPool>) -> Router {
    use discover::saved_views_handler::{
        create_view, delete_view, update_view, SavedViewsState,
    };
    let state = SavedViewsState { pool };
    Router::new()
        .route("/v1/explore/views", post(create_view))
        .route("/v1/explore/views/:id",
            axum::routing::patch(update_view).delete(delete_view))
        .with_state(state)
        .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER.clone(), MakeRequestUuid))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
}
```

- [ ] **Step 3: Mount in `pensieve-bin/src/main.rs`**

Find where `dashboards_write_router(...)` is mounted with `require_role_middleware(Role::Write)` and add the same treatment for `discover_views_write_router`. Pattern:

```rust
let discover_write = pensieve_server::discover_views_write_router(pg_pool.clone())
    .layer(require_role_middleware(Role::Write, auth_state.clone()));
```

- [ ] **Step 4: Compile-check**

```bash
cargo check -p pensieve-server -p pensieve-bin
```

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-server/src/lib.rs crates/pensieve-bin/src/main.rs
git commit -m "feat(discover): mount saved_views routes (read + write)"
```

---

## Task 6: Integration test

**Files:**
- Create: `crates/pensieve-server/tests/discover_saved_views_it.rs`

- [ ] **Step 1: Write the test**

```rust
use http::StatusCode;
use pensieve_server::discover::handler_test_support::seeded_state_two_dbs_two_tables;
use pensieve_server::router;

#[tokio::test]
async fn saved_view_round_trip_and_scopes_search() {
    let state = seeded_state_two_dbs_two_tables().await;
    let app = router(state.clone())
        .merge(pensieve_server::discover_views_write_router(state.pg_pool.clone()));
    let (addr, _shutdown) = pensieve_server::test_support::spawn(app).await;
    let client = reqwest::Client::new();

    // 1) Create
    let create_resp = client
        .post(&format!("http://{addr}/v1/explore/views"))
        .header("authorization", "Bearer test-write-token")
        .header("content-type", "application/json")
        .body(r#"{"name":"prod-only","sources":["obs.*"]}"#)
        .send().await.unwrap();
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let created: serde_json::Value = create_resp.json().await.unwrap();
    let view_id = created["id"].as_str().unwrap().to_string();

    // 2) List
    let list_resp = client
        .get(&format!("http://{addr}/v1/explore/views"))
        .header("authorization", "Bearer test-read-token")
        .send().await.unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let listed: serde_json::Value = list_resp.json().await.unwrap();
    assert!(listed.as_array().unwrap().iter().any(|v| v["name"] == "prod-only"));

    // 3) Use as scope in /v1/explore/search
    let body = serde_json::json!({
        "query": "",
        "scope": { "kind": "view", "view_id": view_id },
    });
    let search_resp = client
        .post(&format!("http://{addr}/v1/explore/search"))
        .header("authorization", "Bearer test-read-token")
        .header("content-type", "application/json")
        .json(&body)
        .send().await.unwrap();
    assert_eq!(search_resp.status(), StatusCode::OK);
    let text = search_resp.text().await.unwrap();
    // Plan should only include obs.* sources (no stg.*).
    let plan: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let plan_sources: Vec<&str> = plan["sources"].as_array().unwrap().iter()
        .map(|s| s["source"].as_str().unwrap()).collect();
    assert!(plan_sources.iter().all(|s| s.starts_with("obs.")));

    // 4) Delete
    let del_resp = client
        .delete(&format!("http://{addr}/v1/explore/views/{view_id}"))
        .header("authorization", "Bearer test-write-token")
        .send().await.unwrap();
    assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

    // 5) Listing the same view by id yields not_found from search
    let stale_body = serde_json::json!({
        "query": "",
        "scope": { "kind": "view", "view_id": view_id },
    });
    let stale_resp = client
        .post(&format!("http://{addr}/v1/explore/search"))
        .header("authorization", "Bearer test-read-token")
        .header("content-type", "application/json")
        .json(&stale_body)
        .send().await.unwrap();
    assert_eq!(stale_resp.status(), StatusCode::NOT_FOUND);
    let v: serde_json::Value = stale_resp.json().await.unwrap();
    assert_eq!(v["error"]["code"], "view_not_found");
}

#[tokio::test]
async fn duplicate_name_yields_409() {
    let state = seeded_state_two_dbs_two_tables().await;
    let app = router(state.clone())
        .merge(pensieve_server::discover_views_write_router(state.pg_pool.clone()));
    let (addr, _shutdown) = pensieve_server::test_support::spawn(app).await;
    let client = reqwest::Client::new();

    let body = r#"{"name":"dup","sources":["obs.*"]}"#;
    let _ = client.post(&format!("http://{addr}/v1/explore/views"))
        .header("authorization", "Bearer test-write-token")
        .header("content-type", "application/json")
        .body(body).send().await.unwrap();

    let r2 = client.post(&format!("http://{addr}/v1/explore/views"))
        .header("authorization", "Bearer test-write-token")
        .header("content-type", "application/json")
        .body(body).send().await.unwrap();

    assert_eq!(r2.status(), StatusCode::CONFLICT);
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test -p pensieve-server --test discover_saved_views_it -- --nocapture
```

Expected: 2 tests pass.

- [ ] **Step 3: Full suite**

```bash
cargo test -p pensieve-server -- --nocapture
```

Expected: all green (including Plan A's tests).

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/tests/discover_saved_views_it.rs
git commit -m "test(discover): integration test for saved views CRUD + search wiring"
```

---

## Self-Review Checklist

**Spec §4.2 coverage:**

- Table schema with `id, name, owner, scope_json, default_columns_json, timestamps` — Task 1 (column names mapped to `sources_json`, `columns_json`).
- `GET /v1/explore/views` — Task 5 (read router).
- `POST /v1/explore/views` — Task 5 (write router).
- `PATCH /v1/explore/views/:id` — Task 5 (write router).
- `DELETE /v1/explore/views/:id` — Task 5 (write router).
- Owner-scoped (RBAC) — Task 2 (all queries filter by `tenant_id + owner_subject`).
- "view = scope only, not saved query" — Task 1 stores only `sources_json`; no `query` or `pills` columns.
- `time_range`, `query`, `per_source_limit` always come from the request body — verified by Task 4 (lookup returns only `Vec<String>` sources) and Task 6's integration test (search request supplies its own empty query alongside the view scope).
- Saved view ViewNotFound error path — Task 4 (lookup returns `None`) → Task 6 Step 1 #5 asserts `view_not_found` HTTP error.

**Placeholder scan:** none.

**Type consistency:**

- `SavedView.sources` is `Vec<String>` everywhere (catalog, handler, lookup).
- `Scope::View { view_id: String }` matches catalog `Uuid::parse_str` in the adapter.
- Handler uses `Extension<Principal>` + `Extension<TenantId>` — same pattern as existing handlers.
