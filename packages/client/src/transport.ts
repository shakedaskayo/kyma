import { KymaAuthError } from "./errors";

export type GetToken = (opts?: { reason: "initial" | "expired" }) => Promise<string>;
export type KymaAuth = { token: string } | { getToken: GetToken };

export interface TransportConfig {
  /** Kyma server URL, e.g. https://kyma.acme.internal */
  endpoint: string;
  auth: KymaAuth;
  /** Default database; per-request override via RequestOpts.database. */
  database?: string;
  /** Injectable for tests / non-browser runtimes. Defaults to globalThis.fetch. */
  fetch?: typeof fetch;
}

export interface RequestOpts extends Omit<RequestInit, "headers"> {
  headers?: HeadersInit;
  database?: string;
}

export interface KymaTransport {
  readonly endpoint: string;
  readonly database?: string;
  request(path: string, opts?: RequestOpts): Promise<Response>;
}

export function createTransport(cfg: TransportConfig): KymaTransport {
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
      const res = await doFetch(path, opts, await token());
      if (res.status !== 401) return res;
      if (!("getToken" in cfg.auth)) {
        throw new KymaAuthError(401, "kyma request failed: 401 — token rejected");
      }
      const fresh = await mint("expired");
      const retry = await doFetch(path, opts, fresh);
      if (retry.status === 401) {
        throw new KymaAuthError(401, "kyma request failed: 401 — token rejected after refresh");
      }
      return retry;
    },
  };
}
