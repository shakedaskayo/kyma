import { eq, and } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getEnv } from '../env.js';
import { badRequest, conflict, notFound, forbidden } from '../lib/errors.js';
import { PLANS, type PlanId } from '@kyma/shared';

export function slugify(input: string): string {
  return input.toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48);
}

export async function listForUser(userId: string) {
  const db = getDb();
  const rows = await db
    .select({
      id: schema.workspaces.id,
      slug: schema.workspaces.slug,
      name: schema.workspaces.name,
      plan: schema.workspaces.plan,
      kind: schema.workspaces.kind,
      planActive: schema.workspaces.planActive,
      mcpEndpoint: schema.workspaces.mcpEndpoint,
      kymaEndpoint: schema.workspaces.kymaEndpoint,
      role: schema.workspaceMembers.role,
      createdAt: schema.workspaces.createdAt,
    })
    .from(schema.workspaceMembers)
    .innerJoin(schema.workspaces, eq(schema.workspaces.id, schema.workspaceMembers.workspaceId))
    .where(eq(schema.workspaceMembers.userId, userId));
  return rows;
}

export async function createWorkspace(userId: string, input: { name: string; slug?: string }) {
  const db = getDb();
  const env = getEnv();
  if (!input.name.trim()) throw badRequest('name is required');

  const baseSlug = input.slug ? slugify(input.slug) : slugify(input.name);
  if (!baseSlug) throw badRequest('slug must contain at least one alphanumeric character');

  // Plan-tier limit (read user's most-permissive workspace plan, default free).
  const existing = await listForUser(userId);
  const userPlan: PlanId = (existing[0]?.plan as PlanId | undefined) ?? 'free';
  if (existing.length >= PLANS[userPlan].maxWorkspaces) {
    throw forbidden(`Plan '${userPlan}' allows at most ${PLANS[userPlan].maxWorkspaces} workspaces.`, 'PLAN_LIMIT');
  }

  let slug = baseSlug;
  let attempt = 0;
  while (true) {
    const [hit] = await db.select({ id: schema.workspaces.id })
      .from(schema.workspaces).where(eq(schema.workspaces.slug, slug)).limit(1);
    if (!hit) break;
    attempt += 1;
    if (attempt > 5) throw conflict(`Slug '${baseSlug}' is taken`);
    slug = `${baseSlug}-${Math.floor(1000 + Math.random() * 9000)}`;
  }

  const [ws] = await db.insert(schema.workspaces).values({
    slug,
    name: input.name.trim(),
    ownerUserId: userId,
    plan: 'free',
    kind: 'shared',
    kymaEndpoint: env.KYMA_ENGINE_BASE_URL,
    // Slice 2 keeps a single shared engine; mcpEndpoint is the same URL for
    // every workspace because the bearer token's tenant_id (= workspace.id)
    // is what selects the workspace at the engine. Slice 3 may switch to
    // per-workspace dedicated clusters with distinct URLs.
    mcpEndpoint: '',  // backfilled below
  }).returning();

  const [updated] = await db.update(schema.workspaces)
    .set({ mcpEndpoint: `${env.KYMA_ENGINE_BASE_URL}/mcp/v1` })
    .where(eq(schema.workspaces.id, ws.id))
    .returning();

  await db.insert(schema.workspaceMembers).values({
    workspaceId: ws.id, userId, role: 'owner',
  });
  return updated;
}

export async function getBySlugForUser(userId: string, slug: string) {
  const db = getDb();
  const [row] = await db
    .select({
      ws: schema.workspaces,
      role: schema.workspaceMembers.role,
    })
    .from(schema.workspaces)
    .innerJoin(schema.workspaceMembers, and(
      eq(schema.workspaceMembers.workspaceId, schema.workspaces.id),
      eq(schema.workspaceMembers.userId, userId),
    ))
    .where(eq(schema.workspaces.slug, slug))
    .limit(1);
  if (!row) throw notFound('Workspace not found');
  return { workspace: row.ws, role: row.role };
}
