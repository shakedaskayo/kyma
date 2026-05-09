// All /api/* requests go to cloud-web's domain, which proxies them to
// cloud-api via Next.js rewrites (see next.config.ts). This keeps cookies
// origin to a single domain so sessions work across the auth flow.
async function call<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    credentials: 'include',
    headers: { 'Content-Type': 'application/json', ...(init?.headers ?? {}) },
  });
  if (!res.ok) {
    const e = await res.json().catch(() => ({ error: { message: `HTTP ${res.status}` } }));
    throw new Error(e.error?.message ?? `HTTP ${res.status}`);
  }
  return res.json();
}

export const api = {
  startGithubAuth: () => { window.location.href = '/api/auth/github/start'; },
  requestMagicLink: (email: string) => call<{ ok: true }>('/api/auth/magic-link/request', {
    method: 'POST', body: JSON.stringify({ email }),
  }),
  exchangeMagicLink: (token: string) =>
    call<{ user: { id: string; email: string; name: string | null } }>(
      '/api/auth/magic-link/exchange',
      { method: 'POST', body: JSON.stringify({ token }) },
    ),
  me: () => call<{ user: { id: string; email: string; name: string | null } }>('/api/auth/me'),
  workspaces: {
    list: () => call<{ workspaces: any[] }>('/api/workspaces'),
    create: (name: string) => call<{ workspace: any }>('/api/workspaces', {
      method: 'POST', body: JSON.stringify({ name }),
    }),
    get: (slug: string) => call<{ workspace: any; role: string }>(`/api/workspaces/${slug}`),
    listTokens: (slug: string) => call<{ tokens: any[] }>(`/api/workspaces/${slug}/tokens`),
    mintToken: (slug: string, name?: string) => call<{
      token: string; prefix: string; id: string; mcpEndpoint: string;
    }>(`/api/workspaces/${slug}/tokens`, { method: 'POST', body: JSON.stringify({ name }) }),
    revokeToken: (slug: string, id: string) => call<{ ok: true }>(
      `/api/workspaces/${slug}/tokens/${id}/revoke`, { method: 'POST' },
    ),
  },
  billing: {
    checkout: (workspaceSlug: string, plan: 'pro' | 'team') => call<{ url: string }>(
      '/api/billing/checkout',
      { method: 'POST', body: JSON.stringify({ workspaceSlug, plan }) },
    ),
    portal: (workspaceSlug: string) => call<{ url: string }>(
      '/api/billing/portal', { method: 'POST', body: JSON.stringify({ workspaceSlug }) },
    ),
    subscription: (slug: string) => call<{
      plan: string; planActive: boolean; trialEndsAt: string | null;
      currentPeriodEnd: string | null; dunningState: string | null;
    }>(`/api/billing/${slug}/subscription`),
  },
};
