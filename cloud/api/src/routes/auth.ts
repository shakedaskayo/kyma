import { Hono } from 'hono';
import { setCookie, getCookie, deleteCookie } from 'hono/cookie';
import { z } from 'zod';
import { getEnv } from '../env.js';
import { badRequest, unauthorized } from '../lib/errors.js';
import { signSessionCookie, SESSION_COOKIE_NAME } from '../lib/sessions.js';
import * as auth from '../services/auth.service.js';
import { sendMagicLinkEmail } from '../services/email.service.js';

const STATE_COOKIE = 'kyma_oauth_state';

export const authRoutes = new Hono();

// GET /api/auth/github/start
authRoutes.get('/github/start', (c) => {
  const env = getEnv();
  const state = auth.newOauthState();
  setCookie(c, STATE_COOKIE, state, {
    httpOnly: true, sameSite: 'Lax', secure: env.NODE_ENV === 'production',
    path: '/', maxAge: 600,
  });
  const redirectUri = `${env.CLOUD_BASE_URL}/api/auth/github/callback`;
  return c.redirect(auth.buildGithubAuthorizeUrl(state, redirectUri));
});

// GET /api/auth/github/callback?code&state
authRoutes.get('/github/callback', async (c) => {
  const env = getEnv();
  const code = c.req.query('code');
  const state = c.req.query('state');
  const cookieState = getCookie(c, STATE_COOKIE);
  if (!code || !state) throw badRequest('Missing code or state');
  if (!cookieState || cookieState !== state) throw unauthorized('OAuth state mismatch (CSRF)');
  deleteCookie(c, STATE_COOKIE, { path: '/' });

  const redirectUri = `${env.CLOUD_BASE_URL}/api/auth/github/callback`;
  const { user } = await auth.exchangeGithubCode(code, redirectUri);

  const jwt = await signSessionCookie({ sub: user.id, email: user.email });
  setCookie(c, SESSION_COOKIE_NAME, jwt, {
    httpOnly: true, sameSite: 'Lax', secure: env.NODE_ENV === 'production',
    path: '/', maxAge: 60 * 60 * 24 * 30,
  });
  return c.redirect(`${env.CLOUD_BASE_URL}/workspaces`);
});

// POST /api/auth/logout
authRoutes.post('/logout', (c) => {
  deleteCookie(c, SESSION_COOKIE_NAME, { path: '/' });
  return c.json({ ok: true });
});

const requestSchema = z.object({ email: z.string().email() });
const exchangeSchema = z.object({ token: z.string().min(32) });

authRoutes.post('/magic-link/request', async (c) => {
  const body = await c.req.json();
  const parsed = requestSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const { link } = await auth.issueMagicLink(parsed.data.email);
  await sendMagicLinkEmail(parsed.data.email, link);
  return c.json({ ok: true });
});

authRoutes.post('/magic-link/exchange', async (c) => {
  const env = getEnv();
  const body = await c.req.json();
  const parsed = exchangeSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const { user } = await auth.exchangeMagicLink(parsed.data.token);
  const jwt = await signSessionCookie({ sub: user.id, email: user.email });
  setCookie(c, SESSION_COOKIE_NAME, jwt, {
    httpOnly: true, sameSite: 'Lax', secure: env.NODE_ENV === 'production',
    path: '/', maxAge: 60 * 60 * 24 * 30,
  });
  return c.json({ user: { id: user.id, email: user.email, name: user.name } });
});
