import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { createHash } from 'node:crypto';
import { eq } from 'drizzle-orm';
import { freshDb } from '../test-setup.js';
import { getDb, closeDb, schema } from './client.js';

describe('schema: api_tokens contract with engine DbAuthBackend', () => {
  beforeAll(async () => { await freshDb(); });
  afterAll(async () => { await closeDb(); });

  it('round-trips a token row with bytea token_hash and csv scopes', async () => {
    const db = getDb();
    const [user] = await db.insert(schema.users).values({
      email: 'a@b.c', githubId: '111', name: 'a',
    }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'demo', name: 'Demo', ownerUserId: user.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/x/mcp/v1',
    }).returning();
    const tokenHash = createHash('sha256').update('kyma_test_token').digest();
    expect(tokenHash.length).toBe(32);
    const [tok] = await db.insert(schema.apiTokens).values({
      tenantId: ws.id,
      workspaceId: ws.id,
      tokenHash,
      scopes: 'read,write',
      name: 't1',
    }).returning();
    expect(tok.scopes).toBe('read,write');
    expect(Buffer.from(tok.tokenHash).equals(tokenHash)).toBe(true);
    const rows = await db
      .select({ tid: schema.apiTokens.tenantId, sc: schema.apiTokens.scopes })
      .from(schema.apiTokens)
      .where(eq(schema.apiTokens.tokenHash, tokenHash));
    expect(rows[0].tid).toBe(ws.id);
    expect(rows[0].sc).toBe('read,write');
  });
});
