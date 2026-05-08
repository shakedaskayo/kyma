import 'server-only';
import * as jose from 'jose';

export const ADMIN_COOKIE = 'kyma_admin_session';
const ISS = 'kyma-admin';

export async function signAdmin(claims: { ghId: string; ghLogin: string }): Promise<string> {
  const secret = new TextEncoder().encode(process.env.SESSION_SECRET ?? '');
  return new jose.SignJWT({ ...claims })
    .setProtectedHeader({ alg: 'HS256' }).setIssuedAt().setExpirationTime('30d').setIssuer(ISS).sign(secret);
}

export async function verifyAdmin(jwt: string): Promise<{ ghId: string; ghLogin: string } | null> {
  try {
    const secret = new TextEncoder().encode(process.env.SESSION_SECRET ?? '');
    const { payload } = await jose.jwtVerify(jwt, secret, { issuer: ISS });
    return { ghId: payload.ghId as string, ghLogin: payload.ghLogin as string };
  } catch { return null; }
}

export function isAllowedAdmin(ghId: string): boolean {
  const ids = (process.env.KYMA_ADMIN_GITHUB_IDS ?? '').split(',').map((s) => s.trim()).filter(Boolean);
  return ids.includes(ghId);
}
