import { createTransport, type KymaTransport, type TransportConfig } from "./transport";
import * as graph from "./graph";
import * as query from "./query";
import * as discover from "./discover";
import * as discoverLive from "./discover-live";
import * as dashboards from "./dashboards";
import * as catalog from "./catalog";
import * as capabilities from "./capabilities";
import * as agentEngine from "./agent-engine";
import * as agentSkills from "./agent-skills";
import * as memory from "./memory";
import * as connectors from "./connectors";
import * as credentials from "./credentials";
import * as auth from "./auth";
import * as setup from "./setup";
import * as oauth from "./oauth";

// ── Curated bind lists ──────────────────────────────────────────────────────
// Each array names the transport-first exported functions in its module.
// Pure helpers (no transport arg) are intentionally absent — they remain
// importable directly from @kyma-ai/client but are NOT attached to the client.

const GRAPH_FNS = [
  "listGraphs",
  "getOverview",
  "getStats",
  "getGraphSchema",
  "getNode",
  "getSubgraph",
  "searchNodes",
  "expandNeighbors",
] as const satisfies readonly (keyof typeof graph)[];

const QUERY_FNS = ["runQuery"] as const satisfies readonly (keyof typeof query)[];

const DISCOVER_FNS = [
  "searchDiscover",
  "listSavedViews",
  "createSavedView",
  "deleteSavedView",
] as const satisfies readonly (keyof typeof discover)[];

// discoverLive's LiveSession takes endpoint+token, not a transport — it predates
// the transport abstraction. It is surfaced as-is under client.discover.LiveSession.

const DASHBOARDS_FNS = [
  "listDashboards",
  "getDashboard",
  "createDashboard",
  "updateDashboard",
  "deleteDashboard",
] as const satisfies readonly (keyof typeof dashboards)[];

const CATALOG_FNS = ["fetchSchema"] as const satisfies readonly (keyof typeof catalog)[];

const CAPABILITIES_FNS = [
  "fetchCapabilities",
] as const satisfies readonly (keyof typeof capabilities)[];

const AGENT_ENGINE_FNS = [
  "listEngines",
  "putEngine",
  "testEngine",
] as const satisfies readonly (keyof typeof agentEngine)[];

const AGENT_SKILLS_FNS = [
  "listSkills",
  "putEnabledSkills",
] as const satisfies readonly (keyof typeof agentSkills)[];

const MEMORY_FNS = [
  "fetchMemoryOverview",
  "queryMemory",
  "getMemorySettings",
  "putMemorySettings",
] as const satisfies readonly (keyof typeof memory)[];

const CONNECTORS_FNS = [
  "getConnectorCatalog",
  "listConnectors",
  "getConnector",
  "createConnector",
  "patchConnector",
  "deleteConnector",
  "pauseConnector",
  "resumeConnector",
  "triggerConnector",
  "listGitHubRepos",
] as const satisfies readonly (keyof typeof connectors)[];

const CREDENTIALS_FNS = [
  "listCredentials",
  "createCredential",
  "getCredential",
  "deleteCredential",
] as const satisfies readonly (keyof typeof credentials)[];

// auth: me + logout take a transport; login + refresh are unauthenticated (no transport)
const AUTH_FNS = ["me", "logout"] as const satisfies readonly (keyof typeof auth)[];

// setup: ingestSample takes a transport; getSetupStatus/getSetupProbe/signup do not
const SETUP_FNS = ["ingestSample"] as const satisfies readonly (keyof typeof setup)[];

// oauth: both functions take a transport
const OAUTH_FNS = [
  "startOAuth",
  "getOAuthFlowStatus",
] as const satisfies readonly (keyof typeof oauth)[];

// ── bind helper ─────────────────────────────────────────────────────────────

type BoundFn<F> = F extends (t: KymaTransport, ...a: infer A) => infer R
  ? (...a: A) => R
  : never;

type BoundModule<M, Keys extends readonly (keyof M)[]> = {
  [K in Keys[number]]: BoundFn<M[K]>;
};

function bind<M extends Record<string, unknown>, Keys extends readonly (keyof M)[]>(
  t: KymaTransport,
  mod: M,
  keys: Keys,
): BoundModule<M, Keys> {
  const out = {} as Record<string, unknown>;
  for (const k of keys) {
    const fn = mod[k] as (...args: unknown[]) => unknown;
    out[k as string] = (...a: unknown[]) => fn(t, ...a);
  }
  return out as BoundModule<M, Keys>;
}

// ── KymaClient interface ─────────────────────────────────────────────────────

export interface KymaClient {
  /** Raw transport — use for one-off requests or advanced use-cases. */
  readonly transport: KymaTransport;

  /** Graph namespace — browse, search, expand nodes & edges. */
  readonly graph: BoundModule<typeof graph, typeof GRAPH_FNS>;

  /** Query namespace — run KQL/SQL over the log/event stores. */
  readonly query: BoundModule<typeof query, typeof QUERY_FNS>;

  /**
   * Discover namespace — streaming search, saved views, and live-tail.
   * `LiveSession` is re-exported as a plain class (no transport binding needed).
   */
  readonly discover: BoundModule<typeof discover, typeof DISCOVER_FNS> & {
    LiveSession: typeof discoverLive.LiveSession;
  };

  /** Dashboards namespace — CRUD for dashboards and panels. */
  readonly dashboards: BoundModule<typeof dashboards, typeof DASHBOARDS_FNS>;

  /** Catalog namespace — schema introspection. */
  readonly catalog: BoundModule<typeof catalog, typeof CATALOG_FNS>;

  /** Capabilities namespace — server feature flags. */
  readonly capabilities: BoundModule<typeof capabilities, typeof CAPABILITIES_FNS>;

  /** Agent namespace — engines, skills. */
  readonly agent: BoundModule<typeof agentEngine, typeof AGENT_ENGINE_FNS> &
    BoundModule<typeof agentSkills, typeof AGENT_SKILLS_FNS>;

  /** Memory namespace — overview, hybrid recall, settings. */
  readonly memory: BoundModule<typeof memory, typeof MEMORY_FNS>;

  /** Connectors namespace — manage data connectors. */
  readonly connectors: BoundModule<typeof connectors, typeof CONNECTORS_FNS>;

  /** Credentials namespace — secret store for PATs, OAuth2 tokens, etc. */
  readonly credentials: BoundModule<typeof credentials, typeof CREDENTIALS_FNS>;

  /** Auth namespace — me, logout (transport-bound). login/refresh stay free functions. */
  readonly auth: BoundModule<typeof auth, typeof AUTH_FNS>;

  /** Setup namespace — ingestSample (transport-bound). getSetupStatus/signup stay free. */
  readonly setup: BoundModule<typeof setup, typeof SETUP_FNS>;

  /** OAuth namespace — start/poll OAuth flows. */
  readonly oauth: BoundModule<typeof oauth, typeof OAUTH_FNS>;

  /**
   * Returns a view of the client with a default database applied to all requests.
   * Caller's explicit per-request `database` still wins (spread order in the
   * wrapped transport ensures `opts.database` overrides the default).
   * Shares the same token cache as the parent — does NOT create a new transport.
   */
  withDatabase(database: string): KymaClient;
}

export type KymaClientConfig = TransportConfig;

// ── factory ──────────────────────────────────────────────────────────────────

function fromTransport(t: KymaTransport): KymaClient {
  return {
    transport: t,

    graph: bind(t, graph, GRAPH_FNS),
    query: bind(t, query, QUERY_FNS),
    discover: {
      ...bind(t, discover, DISCOVER_FNS),
      LiveSession: discoverLive.LiveSession,
    },
    dashboards: bind(t, dashboards, DASHBOARDS_FNS),
    catalog: bind(t, catalog, CATALOG_FNS),
    capabilities: bind(t, capabilities, CAPABILITIES_FNS),
    agent: {
      ...bind(t, agentEngine, AGENT_ENGINE_FNS),
      ...bind(t, agentSkills, AGENT_SKILLS_FNS),
    },
    memory: bind(t, memory, MEMORY_FNS),
    connectors: bind(t, connectors, CONNECTORS_FNS),
    credentials: bind(t, credentials, CREDENTIALS_FNS),
    auth: bind(t, auth, AUTH_FNS),
    setup: bind(t, setup, SETUP_FNS),
    oauth: bind(t, oauth, OAUTH_FNS),

    withDatabase(database: string): KymaClient {
      // Wrap the transport so the default database is applied.
      // opts.database in the caller's per-request options wins because the spread
      // order is `{ database, ...opts }` — a defined opts.database comes last.
      const scoped: KymaTransport = {
        endpoint: t.endpoint,
        database,
        request(path, opts = {}) {
          // Explicit per-request database takes precedence over the scoped default.
          return t.request(path, { database, ...opts });
        },
      };
      return fromTransport(scoped);
    },
  };
}

export function createKymaClient(cfg: KymaClientConfig): KymaClient {
  return fromTransport(createTransport(cfg));
}
