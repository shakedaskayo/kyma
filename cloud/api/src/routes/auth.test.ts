import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';
import { signSessionCookie, SESSION_COOKIE_NAME } from '../lib/sessions.js';
import { buildApp } from '../index.js';

describe('GET /api/auth/me', () => {
  beforeAll(async () => { process.env.SESSION_SECRET = 'a'.repeat(48); await freshDb(); });
  afterAll(async () => { await closeDb(); });

  it('returns user info when cookie is valid', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'me@kyma.dev', name: 'Me' }).returning();
    const jwt = await signSessionCookie({ sub: u.id, email: u.email });
    const res = await buildApp().request('/api/auth/me', {
      headers: { Cookie: `${SESSION_COOKIE_NAME}=${jwt}` },
    });
    expect(res.status).toBe(200);
    const body = await res.json() as any;
    expect(body.user.email).toBe('me@kyma.dev');
  });

  it('returns 401 with no cookie', async () => {
    const res = await buildApp().request('/api/auth/me');
    expect(res.status).toBe(401);
  });
});
