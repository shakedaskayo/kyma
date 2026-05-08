import { eq } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getEnv } from '../env.js';
import { badRequest, unauthorized } from '../lib/errors.js';
import { randomBytesHex } from '../lib/tokens.js';

export function buildGithubAuthorizeUrl(state: string, redirectUri: string): string {
  const params = new URLSearchParams({
    client_id: getEnv().GITHUB_CLIENT_ID,
    redirect_uri: redirectUri,
    scope: 'read:user user:email',
    state,
  });
  return `https://github.com/login/oauth/authorize?${params}`;
}

export function newOauthState(): string { return randomBytesHex(16); }

export async function exchangeGithubCode(code: string, redirectUri: string): Promise<{
  user: typeof schema.users.$inferSelect;
}> {
  const env = getEnv();
  const tokRes = await fetch('https://github.com/login/oauth/access_token', {
    method: 'POST',
    headers: { 'Accept': 'application/json', 'Content-Type': 'application/json' },
    body: JSON.stringify({
      client_id: env.GITHUB_CLIENT_ID,
      client_secret: env.GITHUB_CLIENT_SECRET,
      code,
      redirect_uri: redirectUri,
    }),
  });
  if (!tokRes.ok) throw badRequest('GitHub token exchange failed', 'GH_TOKEN_FAIL');
  const tok = await tokRes.json() as { access_token?: string; error?: string };
  if (!tok.access_token) throw unauthorized(tok.error ?? 'No access token from GitHub');

  const ghHeaders = {
    'Authorization': `Bearer ${tok.access_token}`,
    'User-Agent': 'kyma-cloud',
    'Accept': 'application/vnd.github+json',
  };
  const [profileRes, emailsRes] = await Promise.all([
    fetch('https://api.github.com/user', { headers: ghHeaders }),
    fetch('https://api.github.com/user/emails', { headers: ghHeaders }),
  ]);
  if (!profileRes.ok) throw unauthorized('GitHub profile fetch failed');
  const profile = await profileRes.json() as {
    id: number; login: string; name: string | null; avatar_url: string | null; email: string | null;
  };
  const emails = emailsRes.ok ? await emailsRes.json() as Array<{ email: string; primary: boolean; verified: boolean }> : [];
  const primaryEmail =
    emails.find(e => e.primary && e.verified)?.email
    ?? profile.email
    ?? null;
  if (!primaryEmail) throw badRequest('GitHub account has no verified primary email', 'NO_EMAIL');

  const db = getDb();
  const ghIdStr = String(profile.id);

  // Upsert by github_id; fallback to email match for existing magic-link users.
  let [user] = await db.select().from(schema.users).where(eq(schema.users.githubId, ghIdStr)).limit(1);
  if (!user) {
    [user] = await db.select().from(schema.users).where(eq(schema.users.email, primaryEmail)).limit(1);
    if (user) {
      [user] = await db.update(schema.users).set({
        githubId: ghIdStr,
        avatarUrl: profile.avatar_url,
        name: user.name ?? profile.name ?? profile.login,
        updatedAt: new Date(),
      }).where(eq(schema.users.id, user.id)).returning();
    } else {
      [user] = await db.insert(schema.users).values({
        githubId: ghIdStr,
        email: primaryEmail,
        name: profile.name ?? profile.login,
        avatarUrl: profile.avatar_url,
      }).returning();
    }
  }
  return { user };
}
