// Typed client for the OAuth2 data source connect flow (`/v1/oauth/*`).
// All functions require an authenticated transport.

import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

export interface StartOAuthBody {
  /** Data source type this credential is for (e.g. "googledrive"). */
  data_source_type: string;
  /** Scopes to request (defaults to the provider's defaults server-side). */
  scopes?: string[];
  /** Label for the credential the callback mints. */
  label?: string;
  /** Optional bring-your-own client app (persisted per-tenant, encrypted). */
  client_id?: string;
  client_secret?: string;
}

export interface StartOAuthResult {
  authorize_url: string;
  /** Correlation/CSRF token echoed back by the callback. */
  state: string;
}

export async function startOAuth(
  t: KymaTransport,
  args: { provider: string; body: StartOAuthBody },
): Promise<StartOAuthResult> {
  return handleResponse<StartOAuthResult>(
    await t.request(`/v1/oauth/${encodeURIComponent(args.provider)}/start`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args.body),
    }),
  );
}

export interface OAuthFlowStatus {
  status: "pending" | "consumed" | "completed" | "error";
  credential_id: string | null;
}

/** Poll a flow's status by `state` — the fallback for Tauri / popup-blocked. */
export async function getOAuthFlowStatus(
  t: KymaTransport,
  args: { state: string },
): Promise<OAuthFlowStatus> {
  return handleResponse<OAuthFlowStatus>(
    await t.request(`/v1/oauth/flows/${encodeURIComponent(args.state)}`),
  );
}
