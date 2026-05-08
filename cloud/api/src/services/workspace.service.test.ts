import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';
import { createWorkspace, listForUser, getBySlugForUser, slugify } from './workspace.service.js';

beforeAll(async () => {
  process.env.SESSION_SECRET = 'a'.repeat(48);
  process.env.KYMA_ENGINE_BASE_URL = 'http://e';
  await freshDb();
});
afterAll(async () => { await closeDb(); });

describe('workspace.service', () => {
  it('slugify', () => {
    expect(slugify('My Demo Workspace!')).toBe('my-demo-workspace');
  });

  it('createWorkspace inserts workspace + owner membership + mcpEndpoint', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
    const ws = await createWorkspace(u.id, { name: 'My Demo' });
    expect(ws.slug).toBe('my-demo');
    expect(ws.kind).toBe('shared');
    expect(ws.plan).toBe('free');
    expect(ws.mcpEndpoint).toBe('http://e/mcp/v1');
    const list = await listForUser(u.id);
    expect(list).toHaveLength(1);
    expect(list[0].role).toBe('owner');
  });

  it('rejects creating beyond the free-plan workspace limit', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'b@b.c' }).returning();
    await createWorkspace(u.id, { name: 'first' });
    try {
      await createWorkspace(u.id, { name: 'second' });
      throw new Error('createWorkspace should have rejected with PLAN_LIMIT');
    } catch (err: any) {
      expect(err.code).toBe('PLAN_LIMIT');
      expect(err.statusCode).toBe(403);
      expect(err.message).toMatch(/Plan 'free' allows at most 1/);
    }
  });

  it('appends a random suffix on slug collision', async () => {
    const db = getDb();
    const [u1] = await db.insert(schema.users).values({ email: 'c@b.c' }).returning();
    const [u2] = await db.insert(schema.users).values({ email: 'd@b.c' }).returning();
    await createWorkspace(u1.id, { name: 'shared-name' });
    const ws2 = await createWorkspace(u2.id, { name: 'shared-name' });
    expect(ws2.slug).toMatch(/^shared-name-\d{4}$/);
  });

  it('getBySlugForUser rejects non-members', async () => {
    const db = getDb();
    const [u1] = await db.insert(schema.users).values({ email: 'e@b.c' }).returning();
    const [u2] = await db.insert(schema.users).values({ email: 'f@b.c' }).returning();
    const ws = await createWorkspace(u1.id, { name: 'private' });
    await expect(getBySlugForUser(u2.id, ws.slug)).rejects.toThrow(/Workspace not found/);
  });
});
