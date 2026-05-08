import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';
import { mintMcpToken, listTokens, revokeToken, authenticateForDebug } from './mcp-token.service.js';
import { hashToken } from '../lib/tokens.js';

beforeAll(async () => { process.env.KYMA_ENGINE_BASE_URL = 'http://e'; await freshDb(); });
afterAll(async () => { await closeDb(); });

describe('mcp-token.service', () => {
  it('mints a token whose hash matches the engine contract', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'demo', name: 'Demo', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/x/mcp/v1',
    }).returning();

    const { plain, prefix, id } = await mintMcpToken({
      workspaceId: ws.id, createdByUserId: u.id, name: 'cli',
    });
    expect(plain.startsWith('kyma_')).toBe(true);
    expect(prefix.startsWith('kyma_')).toBe(true);
    expect(id).toBeTruthy();

    // The exact lookup the engine performs (SELECT tenant_id, scopes ... WHERE token_hash = $1):
    const resolved = await authenticateForDebug(plain);
    expect(resolved?.tenantId).toBe(ws.id);
    expect(resolved?.scopes).toEqual(['read', 'write']);
  });

  it('listTokens returns rows with prefix only (no plain)', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'b@b.c' }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'demo2', name: 'Demo2', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/y/mcp/v1',
    }).returning();
    await mintMcpToken({ workspaceId: ws.id, createdByUserId: u.id });
    const list = await listTokens(ws.id);
    expect(list).toHaveLength(1);
    expect(list[0].prefix?.startsWith('kyma_')).toBe(true);
    expect((list[0] as any).tokenHash).toBeUndefined();
  });

  it('revokeToken sets revoked_at and the lookup falls through', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'c@b.c' }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'demo3', name: 'Demo3', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/z/mcp/v1',
    }).returning();
    const { plain, id } = await mintMcpToken({ workspaceId: ws.id, createdByUserId: u.id });
    await revokeToken(ws.id, id);
    expect(await authenticateForDebug(plain)).toBeNull();
  });
});
