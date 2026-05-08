import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getPool, getDb, schema } from '../db/client.js';
import { mintMcpToken } from './mcp-token.service.js';
import { hashToken } from '../lib/tokens.js';

beforeAll(async () => { process.env.KYMA_ENGINE_BASE_URL = 'http://e'; await freshDb(); });
afterAll(async () => { await closeDb(); });

describe('engine DbAuthBackend SQL contract', () => {
  it('mint then run the engine`s exact SELECT', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'eng', name: 'eng', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/x/mcp/v1',
    }).returning();
    const { plain } = await mintMcpToken({ workspaceId: ws.id, createdByUserId: u.id });

    const hash = hashToken(plain);
    // EXACT query from crates/kyma-server/src/auth/db_backend.rs
    const r = await getPool().query(
      `SELECT tenant_id, scopes, subject FROM api_tokens
       WHERE token_hash = $1 AND revoked_at IS NULL`,
      [hash],
    );
    expect(r.rowCount).toBe(1);
    expect(r.rows[0].tenant_id).toBe(ws.id);
    expect(r.rows[0].scopes).toBe('read,write');

    // The engine then does this UPDATE for last_used_at — make sure it works:
    await getPool().query(
      `UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1`,
      [hash],
    );
  });
});
