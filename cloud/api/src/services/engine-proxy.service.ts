import { getEnv } from '../env.js';
import { badRequest } from '../lib/errors.js';

/**
 * Forwards an NDJSON payload to the engine on behalf of the workspace.
 *
 * The engine actually exposes `POST /v1/ingest` (single path) and reads the
 * target database/table from the `X-Database` / `X-Table` headers — not from
 * path segments. The plan's `/v1/ingest/<db>/<table>` form does NOT exist in
 * `crates/kyma-server` and would 404.
 */
export async function ingest(opts: {
  tenantId: string;
  database: string;
  table: string;
  ndjson: string;
  /** Bearer token to forward — typically the same token the CLI presented. */
  token: string;
}): Promise<void> {
  const env = getEnv();
  const url = `${env.KYMA_ENGINE_BASE_URL}/v1/ingest`;
  const res = await fetch(url, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${opts.token}`,
      'Content-Type': 'application/x-ndjson',
      'X-Database': opts.database,
      'X-Table': opts.table,
    },
    body: opts.ndjson,
  });
  if (!res.ok) {
    throw badRequest(
      `engine ingest failed: ${res.status} ${await res.text()}`,
      'ENGINE_FAIL',
    );
  }
}

/**
 * Lists tables in `database` for the caller's tenant.
 *
 * The engine has no per-database `/v1/catalog/databases/:db/tables` route.
 * Instead, `GET /v1/catalog/schema` returns the full catalog tree
 * (databases -> tables -> columns) for the caller's tenant; we filter to
 * the requested database here.
 *
 * Engine response shape (from `crates/kyma-server/src/catalog_handler.rs`):
 *   {
 *     databases: [{
 *       name: string,
 *       tables: [{
 *         name: string,
 *         columns: [{ name: string, type: string, nullable: boolean }]
 *       }]
 *     }]
 *   }
 */
export async function listTables(opts: {
  database: string;
  token: string;
}): Promise<{ tables: Array<{ name: string; columns: string[] }> }> {
  const env = getEnv();
  const url = `${env.KYMA_ENGINE_BASE_URL}/v1/catalog/schema`;
  const res = await fetch(url, {
    headers: { 'Authorization': `Bearer ${opts.token}` },
  });
  if (!res.ok) {
    throw badRequest(`engine list-tables failed: ${res.status}`, 'ENGINE_FAIL');
  }
  const schema = (await res.json()) as {
    databases?: Array<{
      name: string;
      tables?: Array<{
        name: string;
        columns?: Array<{ name: string }>;
      }>;
    }>;
  };
  const db = schema.databases?.find((d) => d.name === opts.database);
  const tables = (db?.tables ?? []).map((t) => ({
    name: t.name,
    columns: (t.columns ?? []).map((c) => c.name),
  }));
  return { tables };
}
