// Typed client for the OAuth2 connector connect flow (`/v1/oauth/*`). Mirrors
// the fetch-helper convention used by `sdk/connectors.ts`.

type BaseArgs = { endpoint: string; token: string; database?: string };

function base(endpoint: string): string {
  return endpoint.replace(/\/$/, "");
}

function headers(token: string, database?: string): Record<string, string> {
  const h: Record<string, string> = {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  };
  if (database) h["x-database"] = database;
  return h;
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
  return res.json() as Promise<T>;
}

export interface StartOAuthBody {
  /** Connector type this credential is for (e.g. "googledrive"). */
  connector_type: string;
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
  args: BaseArgs & { provider: string; body: StartOAuthBody },
): Promise<StartOAuthResult> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/oauth/${encodeURIComponent(args.provider)}/start`,
    {
      method: "POST",
      headers: headers(args.token, args.database),
      body: JSON.stringify(args.body),
    },
  );
  return handleResponse<StartOAuthResult>(res);
}

export interface OAuthFlowStatus {
  status: "pending" | "consumed" | "completed" | "error";
  credential_id: string | null;
}

/** Poll a flow's status by `state` — the fallback for Tauri / popup-blocked. */
export async function getOAuthFlowStatus(
  args: BaseArgs & { state: string },
): Promise<OAuthFlowStatus> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/oauth/flows/${encodeURIComponent(args.state)}`,
    { headers: headers(args.token, args.database) },
  );
  return handleResponse<OAuthFlowStatus>(res);
}
