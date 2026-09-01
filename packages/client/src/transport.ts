import { errorFromResponse } from "./errors";

export type GetToken = (opts?: { reason: "initial" | "expired" }) => Promise<string>;
export type PensieveAuth = { token: string } | { getToken: GetToken };

export interface TransportConfig {
  /** Pensieve server URL, e.g. https://pensieve.acme.internal */
  endpoint: string;
  auth: PensieveAuth;
  /** Default database; per-request override via RequestOpts.database. */
  database?: string;
  /** Injectable for tests / non-browser runtimes. Defaults to globalThis.fetch. */
  fetch?: typeof fetch;
}

export interface RequestOpts extends Omit<RequestInit, "headers"> {
  headers?: HeadersInit;
  database?: string;
}

export interface PensieveTransport {
  readonly endpoint: string;
  readonly database?: string;
  request(path: string, opts?: RequestOpts): Promise<Response>;
}

/**
 * Decode the `exp` claim from a JWT payload. Returns the expiry in seconds
 * since epoch, or null if the token is not a JWT or cannot be decoded.
 * Never throws.
 */
function jwtExpiry(token: string): number | null {
  try {
    const parts = token.split(".");
    if (parts.length !== 3) return null;
    // base64url → base64 → decode
    const b64 = parts[1].replace(/-/g, "+").replace(/_/g, "/");
    const padded = b64.padEnd(b64.length + ((4 - (b64.length % 4)) % 4), "=");
    const payload = JSON.parse(atob(padded)) as { exp?: unknown };
    return typeof payload.exp === "number" ? payload.exp : null;
  } catch {
    return null;
  }
}

/** Returns true when a getToken-based JWT should be proactively refreshed. */
function shouldProactiveRefresh(tok: string | null): boolean {
  if (!tok) return false;
  const exp = jwtExpiry(tok);
  if (exp === null) return false;
  // Refresh if expiry is within 60 seconds of now (or already past)
  return exp - Math.floor(Date.now() / 1000) < 60;
}

export function createTransport(cfg: TransportConfig): PensieveTransport {
  const base = cfg.endpoint.replace(/\/$/, "");
  const fetchImpl = cfg.fetch ?? globalThis.fetch.bind(globalThis);

  let cachedToken: string | null = "token" in cfg.auth ? cfg.auth.token : null;
  let mintInFlight: Promise<string> | null = null;

  function mint(reason: "initial" | "expired"): Promise<string> {
    if (!("getToken" in cfg.auth)) return Promise.resolve(cachedToken ?? "");
    if (mintInFlight) return mintInFlight;
    mintInFlight = cfg.auth
      .getToken({ reason })
      .then((tok) => {
        cachedToken = tok;
        return tok;
      })
      .finally(() => {
        mintInFlight = null;
      });
    return mintInFlight;
  }

  async function token(): Promise<string> {
    // Fix 2: proactive JWT refresh when token is near expiry (getToken auth only)
    if ("getToken" in cfg.auth && shouldProactiveRefresh(cachedToken)) {
      return mint("expired");
    }
    if (cachedToken) return cachedToken;
    return mint("initial");
  }

  async function doFetch(path: string, opts: RequestOpts, tok: string): Promise<Response> {
    const url = path.startsWith("http") ? path : `${base}${path.startsWith("/") ? path : `/${path}`}`;
    const headers = new Headers(opts.headers);
    if (tok && !headers.has("authorization")) headers.set("authorization", `Bearer ${tok}`);
    const db = opts.database ?? cfg.database;
    if (db && !headers.has("x-database")) headers.set("x-database", db);
    const { database: _db, ...init } = opts;
    const res = await fetchImpl(url, { ...init, headers });
    // Legacy servers (≤0.0.2) served the SPA for unknown /v1/* paths.
    if (res.ok && url.includes("/v1/") && (res.headers.get("content-type") ?? "").includes("text/html")) {
      return new Response(JSON.stringify({ error: "endpoint not available on this server" }), {
        status: 404,
        headers: { "content-type": "application/json" },
      });
    }
    return res;
  }

  return {
    endpoint: base,
    database: cfg.database,
    async request(path, opts = {}) {
      // Fix 2: capture the token actually used for this request
      const used = await token();
      const res = await doFetch(path, opts, used);
      if (res.status !== 401) return res;

      // Fix 3: static token — use errorFromResponse to preserve requestId
      if (!("getToken" in cfg.auth)) {
        throw await errorFromResponse(res);
      }

      // Fix 1: staggered-401 guard — if cachedToken was refreshed by a concurrent
      // request while we were in-flight, reuse that fresh token without a new mint
      let fresh: string;
      if (cachedToken && cachedToken !== used) {
        fresh = cachedToken;
      } else {
        fresh = await mint("expired");
      }

      const retry = await doFetch(path, opts, fresh);
      if (retry.status === 401) {
        // Fix 3: preserve requestId from retry response
        throw await errorFromResponse(retry);
      }
      return retry;
    },
  };
}
