import { Hono } from 'hono';
import { and, eq, gte } from 'drizzle-orm';
import { sessionMiddleware } from '../middleware/session.js';
import { getDb, schema } from '../db/client.js';
import * as ws from '../services/workspace.service.js';

export const usageRoutes = new Hono();
usageRoutes.use('*', sessionMiddleware);

usageRoutes.get('/:slug/daily', async (c) => {
  const u = c.get('user');
  const { workspace } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  const since = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000);
  const rows = await getDb().select().from(schema.usageDaily)
    .where(and(eq(schema.usageDaily.workspaceId, workspace.id), gte(schema.usageDaily.day, since)));
  return c.json({ usage: rows });
});
