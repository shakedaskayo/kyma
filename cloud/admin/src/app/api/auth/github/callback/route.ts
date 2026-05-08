import { NextResponse } from 'next/server';
import { cookies } from 'next/headers';
import { signAdmin, ADMIN_COOKIE, isAllowedAdmin } from '@/lib/admin-session';

export async function GET(req: Request) {
  const url = new URL(req.url);
  const code = url.searchParams.get('code');
  const state = url.searchParams.get('state');
  const cookieJar = await cookies();
  const stored = cookieJar.get('kyma_admin_oauth_state')?.value;
  if (!code || !state || stored !== state) {
    return new NextResponse('OAuth state mismatch', { status: 401 });
  }
  cookieJar.delete('kyma_admin_oauth_state');

  const tokRes = await fetch('https://github.com/login/oauth/access_token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({
      client_id: process.env.GITHUB_CLIENT_ID,
      client_secret: process.env.GITHUB_CLIENT_SECRET,
      code,
      redirect_uri: `${process.env.ADMIN_BASE_URL}/api/auth/github/callback`,
    }),
  });
  const tok = await tokRes.json() as { access_token?: string };
  if (!tok.access_token) return new NextResponse('Token exchange failed', { status: 401 });

  const profRes = await fetch('https://api.github.com/user', {
    headers: { Authorization: `Bearer ${tok.access_token}`, 'User-Agent': 'kyma-admin' },
  });
  const profile = await profRes.json() as { id: number; login: string };
  if (!isAllowedAdmin(String(profile.id))) {
    return new NextResponse('Not on the admin allowlist', { status: 403 });
  }
  const jwt = await signAdmin({ ghId: String(profile.id), ghLogin: profile.login });
  cookieJar.set(ADMIN_COOKIE, jwt, {
    httpOnly: true, sameSite: 'lax', path: '/',
    maxAge: 60 * 60 * 24 * 30, secure: process.env.NODE_ENV === 'production',
  });
  return NextResponse.redirect(`${process.env.ADMIN_BASE_URL}/workspaces`);
}
