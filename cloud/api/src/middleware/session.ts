import { createMiddleware } from 'hono/factory';
import { getCookie } from 'hono/cookie';
import { eq } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { unauthorized } from '../lib/errors.js';
import { verifySessionCookie, SESSION_COOKIE_NAME } from '../lib/sessions.js';

export interface SessionContext {
  userId: string;
  email: string;
  name: string | null;
}

export const sessionMiddleware = createMiddleware<{
  Variables: { user: SessionContext };
}>(async (c, next) => {
  const cookie = getCookie(c, SESSION_COOKIE_NAME);
  if (!cookie) throw unauthorized('Not signed in');
  let claims;
  try { claims = await verifySessionCookie(cookie); }
  catch { throw unauthorized('Invalid session'); }
  const db = getDb();
  const [u] = await db.select().from(schema.users)
    .where(eq(schema.users.id, claims.sub)).limit(1);
  if (!u) throw unauthorized('User not found');
  c.set('user', { userId: u.id, email: u.email, name: u.name });
  await next();
});
