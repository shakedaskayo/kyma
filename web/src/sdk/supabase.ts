// Lazy Supabase client + session helpers.
//
// The client is created on demand from the project URL + anon key persisted
// in the pensieve session store (put there by the login page after
// `fetchAuthConfig`). supabase-js persists its own refresh token in
// localStorage, so a page reload can refresh the access token without
// re-login. Background auto-refresh is disabled — pensieve's fetch wrapper
// drives refresh on 401 instead (see auth-fetch.ts).

import type { SupabaseClient } from "@supabase/supabase-js";
import { useSession } from "./session";

let client: SupabaseClient | null = null;
let clientKey = "";

/**
 * Get (or create) the Supabase client for the current session's project.
 * Returns null when the session has no Supabase configuration.
 */
export async function getSupabase(): Promise<SupabaseClient | null> {
  const { supabaseUrl, supabaseAnonKey } = useSession.getState();
  if (!supabaseUrl || !supabaseAnonKey) return null;
  const key = `${supabaseUrl}|${supabaseAnonKey}`;
  if (client && clientKey === key) return client;
  const { createClient } = await import("@supabase/supabase-js");
  client = createClient(supabaseUrl, supabaseAnonKey, {
    auth: {
      persistSession: true,
      autoRefreshToken: false,
      detectSessionInUrl: true,
    },
  });
  clientKey = key;
  return client;
}

/**
 * Refresh the Supabase access token (refreshing the underlying session if
 * expired), push it into the pensieve session store, and return it.
 * Returns null when there is no recoverable Supabase session.
 */
export async function refreshSupabaseToken(): Promise<string | null> {
  const sb = await getSupabase();
  if (!sb) return null;
  // getSession returns the cached session; refresh explicitly so a stale
  // access token (the reason we got a 401) is actually rotated.
  const { data, error } = await sb.auth.refreshSession();
  const token = data.session?.access_token ?? null;
  if (error || !token) return null;
  useSession.getState().set({ token });
  return token;
}

/** Sign out of Supabase (best-effort) — callers also reset the pensieve session. */
export async function supabaseSignOut(): Promise<void> {
  const sb = await getSupabase();
  if (!sb) return;
  await sb.auth.signOut().catch(() => {});
}
