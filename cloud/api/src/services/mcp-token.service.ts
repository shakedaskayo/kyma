import { and, eq, isNull } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { generateMcpToken, hashToken } from '../lib/tokens.js';
import { badRequest, notFound } from '../lib/errors.js';

export interface MintInput {
  workspaceId: string;        // = tenant_id used by engine
  createdByUserId: string;
  name?: string;
  scopes?: Array<'read' | 'write' | 'admin'>;
}

export async function mintMcpToken(input: MintInput): Promise<{
  plain: string; prefix: string; id: string;
}> {
  const db = getDb();
  const scopes = (input.scopes && input.scopes.length ? input.scopes : ['read', 'write']).join(',');
  const { plain, hash, prefix } = generateMcpToken();
  const [row] = await db.insert(schema.apiTokens).values({
    tenantId: input.workspaceId,
    workspaceId: input.workspaceId,
    tokenHash: hash,
    scopes,
    name: input.name?.slice(0, 128) ?? 'mcp',
    prefix,
    createdByUserId: input.createdByUserId,
  }).returning({ id: schema.apiTokens.id });
  return { plain, prefix, id: row.id };
}

export async function listTokens(workspaceId: string) {
  const db = getDb();
  return db.select({
    id: schema.apiTokens.id,
    name: schema.apiTokens.name,
    prefix: schema.apiTokens.prefix,
    scopes: schema.apiTokens.scopes,
    createdAt: schema.apiTokens.createdAt,
    lastUsedAt: schema.apiTokens.lastUsedAt,
    revokedAt: schema.apiTokens.revokedAt,
  })
  .from(schema.apiTokens)
  .where(eq(schema.apiTokens.workspaceId, workspaceId));
}

export async function revokeToken(workspaceId: string, tokenId: string) {
  const db = getDb();
  const [updated] = await db
    .update(schema.apiTokens)
    .set({ revokedAt: new Date() })
    .where(and(
      eq(schema.apiTokens.id, tokenId),
      eq(schema.apiTokens.workspaceId, workspaceId),
      isNull(schema.apiTokens.revokedAt),
    ))
    .returning({ id: schema.apiTokens.id });
  if (!updated) throw notFound('Token not found or already revoked');
}

/**
 * Returns the principal for a given plain token, or null. Mirrors the
 * engine's DbAuthBackend exactly so we can sanity-check from the cloud side
 * (e.g. in admin tools).
 */
export async function authenticateForDebug(plain: string): Promise<
  { tenantId: string; scopes: string[] } | null
> {
  const db = getDb();
  const hash = hashToken(plain);
  const [row] = await db
    .select({ tenantId: schema.apiTokens.tenantId, scopes: schema.apiTokens.scopes })
    .from(schema.apiTokens)
    .where(and(eq(schema.apiTokens.tokenHash, hash), isNull(schema.apiTokens.revokedAt)))
    .limit(1);
  if (!row) return null;
  return { tenantId: row.tenantId, scopes: row.scopes.split(',').map((s) => s.trim()) };
}
