import { NextResponse } from 'next/server';
import { cookies } from 'next/headers';

export async function GET() {
  const state = crypto.randomUUID();
  (await cookies()).set('kyma_admin_oauth_state', state, {
    httpOnly: true, sameSite: 'lax', path: '/', maxAge: 600,
  });
  const params = new URLSearchParams({
    client_id: process.env.GITHUB_CLIENT_ID ?? '',
    redirect_uri: `${process.env.ADMIN_BASE_URL}/api/auth/github/callback`,
    scope: 'read:user',
    state,
  });
  return NextResponse.redirect(`https://github.com/login/oauth/authorize?${params}`);
}
