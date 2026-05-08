import 'server-only';
import { cookies } from 'next/headers';
import * as jose from 'jose';

const SESSION_COOKIE = 'kyma_session';
const ISS = 'kyma-cloud';

export async function getCurrentUser(): Promise<{ id: string; email: string } | null> {
  const c = await cookies();
  const jwt = c.get(SESSION_COOKIE)?.value;
  if (!jwt) return null;
  try {
    const secret = new TextEncoder().encode(process.env.SESSION_SECRET ?? '');
    const { payload } = await jose.jwtVerify(jwt, secret, { issuer: ISS });
    return { id: payload.sub as string, email: payload.email as string };
  } catch { return null; }
}
