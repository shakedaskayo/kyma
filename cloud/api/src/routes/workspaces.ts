import { Hono } from 'hono';
import { z } from 'zod';
import { sessionMiddleware } from '../middleware/session.js';
import { AppError, badRequest, forbidden } from '../lib/errors.js';
import type { HonoEnv } from '../lib/hono-env.js';
import * as ws from '../services/workspace.service.js';
import * as tok from '../services/mcp-token.service.js';
import { authenticateForDebug } from '../services/mcp-token.service.js';
import * as engine from '../services/engine-proxy.service.js';

export const workspaceRoutes = new Hono<HonoEnv>();
workspaceRoutes.use('*', sessionMiddleware);

const createSchema = z.object({
  name: z.string().min(1).max(255),
  slug: z.string().min(1).max(64).optional(),
});
const tokenCreateSchema = z.object({
  name: z.string().max(128).optional(),
  scopes: z.array(z.enum(['read', 'write', 'admin'])).optional(),
});

workspaceRoutes.get('/', async (c) => {
  const u = c.get('user');
  const list = await ws.listForUser(u.userId);
  return c.json({ workspaces: list });
});

workspaceRoutes.post('/', async (c) => {
  const u = c.get('user');
  const body = await c.req.json();
  const parsed = createSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const created = await ws.createWorkspace(u.userId, parsed.data);
  return c.json({ workspace: created }, 201);
});

workspaceRoutes.get('/:slug', async (c) => {
  const u = c.get('user');
  const { workspace, role } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  return c.json({ workspace, role });
});

workspaceRoutes.get('/:slug/tokens', async (c) => {
  const u = c.get('user');
  const { workspace } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  return c.json({ tokens: await tok.listTokens(workspace.id) });
});

workspaceRoutes.post('/:slug/tokens', async (c) => {
  const u = c.get('user');
  const { workspace, role } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  if (!['owner', 'admin'].includes(role)) throw forbidden('Only owner or admin can mint tokens');
  const body = await c.req.json().catch(() => ({}));
  const parsed = tokenCreateSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const minted = await tok.mintMcpToken({
    workspaceId: workspace.id,
    createdByUserId: u.userId,
    name: parsed.data.name,
    scopes: parsed.data.scopes,
  });
  return c.json({
    token: minted.plain,           // returned ONCE — never again
    prefix: minted.prefix,
    id: minted.id,
    mcpEndpoint: workspace.mcpEndpoint,
  }, 201);
});

workspaceRoutes.post('/:slug/tokens/:id/revoke', async (c) => {
  const u = c.get('user');
  const { workspace, role } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  if (!['owner', 'admin'].includes(role)) throw forbidden('Only owner or admin can revoke tokens');
  await tok.revokeToken(workspace.id, c.req.param('id'));
  return c.json({ ok: true });
});

// ---------------------------------------------------------------------------
// Token-authenticated routes (engine-proxy surface)
// ---------------------------------------------------------------------------
//
// These are mounted at the same prefix as `workspaceRoutes` but use bearer-
// token auth (the `Authorization: Bearer kyma_*` token the CLI minted) rather
// than the session cookie. Hono routes by method+path, so the two sub-routers
// coexist cleanly.

export const workspaceTokenRoutes = new Hono();

async function authWithToken(c: any): Promise<{ tenantId: string; token: string }> {
  const h = c.req.header('Authorization');
  if (!h?.startsWith('Bearer ')) throw new AppError(401, 'UNAUTH', 'Missing token');
  const token = h.slice(7);
  const principal = await authenticateForDebug(token);
  if (!principal) throw new AppError(401, 'UNAUTH', 'Invalid token');
  return { tenantId: principal.tenantId, token };
}

workspaceTokenRoutes.post('/:slug/ingest/:db/:table', async (c) => {
  const { token } = await authWithToken(c);
  const ndjson = await c.req.raw.text();
  await engine.ingest({
    tenantId: '', // engine derives from token
    database: c.req.param('db'),
    table: c.req.param('table'),
    ndjson,
    token,
  });
  return c.json({ ok: true });
});

workspaceTokenRoutes.get('/:slug/databases/:db/tables', async (c) => {
  const { token } = await authWithToken(c);
  const r = await engine.listTables({ database: c.req.param('db'), token });
  return c.json(r);
});
