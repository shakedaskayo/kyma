import { Hono } from 'hono';
import { z } from 'zod';
import { sessionMiddleware } from '../middleware/session.js';
import { badRequest, forbidden } from '../lib/errors.js';
import * as ws from '../services/workspace.service.js';
import * as tok from '../services/mcp-token.service.js';

export const workspaceRoutes = new Hono();
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
