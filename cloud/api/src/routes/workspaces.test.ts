import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';
import { signSessionCookie, SESSION_COOKIE_NAME } from '../lib/sessions.js';
import { buildApp } from '../index.js';

let cookie = '';
let userId = '';

beforeAll(async () => {
  process.env.SESSION_SECRET = 'a'.repeat(48);
  process.env.KYMA_ENGINE_BASE_URL = 'http://engine.local';
  await freshDb();
  const db = getDb();
  const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
  userId = u.id;
  cookie = `${SESSION_COOKIE_NAME}=${await signSessionCookie({ sub: u.id, email: u.email })}`;
});
afterAll(async () => { await closeDb(); });

describe('workspaces routes', () => {
  it('POST creates a workspace with the owning member', async () => {
    const res = await buildApp().request('/api/workspaces', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Cookie: cookie },
      body: JSON.stringify({ name: 'Acme' }),
    });
    expect(res.status).toBe(201);
    const body = await res.json() as any;
    expect(body.workspace.slug).toBe('acme');
    expect(body.workspace.mcpEndpoint).toMatch(/\/mcp\/v1$/);
  });

  it('POST tokens mints a one-time token', async () => {
    const res = await buildApp().request('/api/workspaces/acme/tokens', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Cookie: cookie },
      body: JSON.stringify({ name: 'cli' }),
    });
    expect(res.status).toBe(201);
    const body = await res.json() as any;
    expect(body.token).toMatch(/^kyma_/);
    expect(body.mcpEndpoint).toMatch(/\/mcp\/v1$/);
  });
});
