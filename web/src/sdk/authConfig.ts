// Runtime auth discovery (`GET /v1/auth/config`, unauthenticated).
//
// The SPA ships with zero build-time configuration — the server tells it at
// runtime whether to render pensieve's password login or the Supabase flow (and
// with which project URL / anon key / OAuth providers).

export interface PasswordAuthConfig {
  provider: "password";
}

export interface SupabaseAuthConfig {
  provider: "supabase";
  supabase_url: string;
  supabase_anon_key: string;
  oauth_providers: string[];
}

export type AuthConfig = PasswordAuthConfig | SupabaseAuthConfig;

/**
 * Fetch the server's auth configuration. Falls back to password mode for
 * older servers that don't expose the endpoint (404) or on any error — the
 * password form is always a safe default.
 */
export async function fetchAuthConfig(endpoint: string): Promise<AuthConfig> {
  try {
    const res = await fetch(`${endpoint.replace(/\/$/, "")}/v1/auth/config`);
    if (!res.ok) return { provider: "password" };
    const body = (await res.json()) as Partial<SupabaseAuthConfig> & { provider?: string };
    if (body.provider === "supabase" && body.supabase_url && body.supabase_anon_key) {
      return {
        provider: "supabase",
        supabase_url: body.supabase_url,
        supabase_anon_key: body.supabase_anon_key,
        oauth_providers: body.oauth_providers ?? [],
      };
    }
    return { provider: "password" };
  } catch {
    return { provider: "password" };
  }
}
