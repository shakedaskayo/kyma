import { createTransport, type PensieveTransport, type TransportConfig } from "./transport";
import * as graph from "./graph";
import * as query from "./query";
import * as discover from "./discover";
import * as search from "./search";
import * as discoverLive from "./discover-live";
import * as dashboards from "./dashboards";
import * as catalog from "./catalog";
import * as capabilities from "./capabilities";
import * as agentEngine from "./agent-engine";
import * as agentSkills from "./agent-skills";
import * as memory from "./memory";
import * as datasources from "./datasources";
import * as credentials from "./credentials";
import * as auth from "./auth";
import * as setup from "./setup";
import * as oauth from "./oauth";
import * as artifacts from "./artifacts";

// ── Bind-coverage type guard ─────────────────────────────────────────────────
// TransportFirstKeys<M> resolves to the union of keys in M whose value is a
// function whose first parameter extends PensieveTransport. This lets us write
// compile-time assertions that every transport-first export appears in its
// *_FNS list — a forgotten entry produces a type error at build time.
//
// AssertSameKeys<A, B> is "never" when the two string unions differ.
// Usage: `type _Assert = AssertSameKeys<typeof MY_FNS[number], TransportFirstKeys<typeof myMod>>`
// If A ⊄ B or B ⊄ A the conditional resolves to `never`, and the `type`
// declaration itself won't error — but we pass it through `void` in an
// immediately-unused variable so the compiler flags the never.

type TransportFirstKeys<M> = {
  [K in keyof M]: M[K] extends (t: PensieveTransport, ...args: never[]) => unknown ? K : never;
}[keyof M] &
  string;

// AssertSameKeys<A, B> resolves to `true` when the string unions are identical.
// When they differ it resolves to a descriptive tuple — used by Expect<T extends true>
// to produce a TS2344 error that names the offending keys.
type AssertSameKeys<A extends string, B extends string> =
  [A] extends [B]
    ? ([B] extends [A] ? true : ["FNS list is MISSING transport-first keys:", Exclude<B, A>])
    : ["FNS list has EXTRA keys not in module:", Exclude<A, B>];

// Expect<T extends true> — used to surface AssertSameKeys failures as compiler errors.
// noUnusedLocals is satisfied because the type aliases appear in a `declare const`.
type Expect<T extends true> = T extends true ? true : never;

// ── Curated bind lists ──────────────────────────────────────────────────────
// Each array names the transport-first exported functions in its module.
// Pure helpers (no transport arg) are intentionally absent — they remain
// importable directly from @pensieve-ai/client but are NOT attached to the client.

const GRAPH_FNS = [
  "listGraphs",
  "getOverview",
  "getStats",
  "getGraphSchema",
  "getNode",
  "getSubgraph",
  "searchNodes",
  "expandNeighbors",
  "exportGraph",
] as const satisfies readonly (keyof typeof graph)[];

const QUERY_FNS = ["runQuery"] as const satisfies readonly (keyof typeof query)[];

const SEARCH_FNS = ["search"] as const satisfies readonly (keyof typeof search)[];

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
  "memorySourceSummary",
] as const satisfies readonly (keyof typeof memory)[];

const DATASOURCES_FNS = [
  "getDataSourceCatalog",
  "listDataSources",
  "getDataSource",
  "createDataSource",
  "patchDataSource",
  "deleteDataSource",
  "pauseDataSource",
  "resumeDataSource",
  "triggerDataSource",
  "listGitHubRepos",
  "listDataSourceWatchers",
  "getWatcherSettings",
  "updateWatcherSettings",
] as const satisfies readonly (keyof typeof datasources)[];

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

const ARTIFACTS_FNS = [
  "fetchArtifactByPath",
] as const satisfies readonly (keyof typeof artifacts)[];

// ── Bind-coverage assertions ─────────────────────────────────────────────────
// Each Expect<AssertSameKeys<...>> asserts that a *_FNS list covers EXACTLY the
// transport-first exports of its module (no more, no fewer).
// When the sets differ the compiler emits TS2344 naming the missing/extra keys.
// `declare const` avoids runtime cost and noUnusedLocals errors.

declare const _bindGuard: [
  Expect<AssertSameKeys<typeof GRAPH_FNS[number],         TransportFirstKeys<typeof graph>>>,
  Expect<AssertSameKeys<typeof QUERY_FNS[number],         TransportFirstKeys<typeof query>>>,
  Expect<AssertSameKeys<typeof SEARCH_FNS[number],        TransportFirstKeys<typeof search>>>,
  Expect<AssertSameKeys<typeof DISCOVER_FNS[number],      TransportFirstKeys<typeof discover>>>,
  Expect<AssertSameKeys<typeof DASHBOARDS_FNS[number],    TransportFirstKeys<typeof dashboards>>>,
  Expect<AssertSameKeys<typeof CATALOG_FNS[number],       TransportFirstKeys<typeof catalog>>>,
  Expect<AssertSameKeys<typeof CAPABILITIES_FNS[number],  TransportFirstKeys<typeof capabilities>>>,
  Expect<AssertSameKeys<typeof AGENT_ENGINE_FNS[number],  TransportFirstKeys<typeof agentEngine>>>,
  Expect<AssertSameKeys<typeof AGENT_SKILLS_FNS[number],  TransportFirstKeys<typeof agentSkills>>>,
  Expect<AssertSameKeys<typeof MEMORY_FNS[number],        TransportFirstKeys<typeof memory>>>,
  Expect<AssertSameKeys<typeof DATASOURCES_FNS[number],    TransportFirstKeys<typeof datasources>>>,
  Expect<AssertSameKeys<typeof CREDENTIALS_FNS[number],   TransportFirstKeys<typeof credentials>>>,
  Expect<AssertSameKeys<typeof AUTH_FNS[number],          TransportFirstKeys<typeof auth>>>,
  Expect<AssertSameKeys<typeof SETUP_FNS[number],         TransportFirstKeys<typeof setup>>>,
  Expect<AssertSameKeys<typeof OAUTH_FNS[number],         TransportFirstKeys<typeof oauth>>>,
  Expect<AssertSameKeys<typeof ARTIFACTS_FNS[number],     TransportFirstKeys<typeof artifacts>>>,
];

// ── bind helper ─────────────────────────────────────────────────────────────

type BoundFn<F> = F extends (t: PensieveTransport, ...a: infer A) => infer R
  ? (...a: A) => R
  : never;

type BoundModule<M, Keys extends readonly (keyof M)[]> = {
  [K in Keys[number]]: BoundFn<M[K]>;
};

function bind<M extends Record<string, unknown>, Keys extends readonly (keyof M)[]>(
  t: PensieveTransport,
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

// ── PensieveClient interface ─────────────────────────────────────────────────────

export interface PensieveClient {
  /** Raw transport — use for one-off requests or advanced use-cases. */
  readonly transport: PensieveTransport;

  /** Graph namespace — browse, search, expand nodes & edges. */
  readonly graph: BoundModule<typeof graph, typeof GRAPH_FNS>;

  /** Query namespace — run KQL/SQL over the log/event stores. */
  readonly query: BoundModule<typeof query, typeof QUERY_FNS>;
  /** Unified hybrid search (lexical + vector + RRF) across sources in scope. */
  readonly search: BoundModule<typeof search, typeof SEARCH_FNS>;

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

  /** Data sources namespace — manage data sources. */
  readonly datasources: BoundModule<typeof datasources, typeof DATASOURCES_FNS>;

  /** Credentials namespace — secret store for PATs, OAuth2 tokens, etc. */
  readonly credentials: BoundModule<typeof credentials, typeof CREDENTIALS_FNS>;

  /** Auth namespace — me, logout (transport-bound). login/refresh stay free functions. */
  readonly auth: BoundModule<typeof auth, typeof AUTH_FNS>;

  /** Setup namespace — ingestSample (transport-bound). getSetupStatus/signup stay free. */
  readonly setup: BoundModule<typeof setup, typeof SETUP_FNS>;

  /** OAuth namespace — start/poll OAuth flows. */
  readonly oauth: BoundModule<typeof oauth, typeof OAUTH_FNS>;

  /** Artifacts namespace — fetch stored object-store artifacts (e.g. CI log files) by key. */
  readonly artifacts: BoundModule<typeof artifacts, typeof ARTIFACTS_FNS>;

  /**
   * Returns a view of the client with a default database applied to all requests.
   * Caller's explicit per-request `database` still wins (spread order in the
   * wrapped transport ensures `opts.database` overrides the default).
   * Shares the same token cache as the parent — does NOT create a new transport.
   */
  withDatabase(database: string): PensieveClient;
}

export type PensieveClientConfig = TransportConfig;

// ── factory ──────────────────────────────────────────────────────────────────

function fromTransport(t: PensieveTransport): PensieveClient {
  return {
    transport: t,

    graph: bind(t, graph, GRAPH_FNS),
    query: bind(t, query, QUERY_FNS),
    search: bind(t, search, SEARCH_FNS),
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
    datasources: bind(t, datasources, DATASOURCES_FNS),
    credentials: bind(t, credentials, CREDENTIALS_FNS),
    auth: bind(t, auth, AUTH_FNS),
    setup: bind(t, setup, SETUP_FNS),
    oauth: bind(t, oauth, OAUTH_FNS),
    artifacts: bind(t, artifacts, ARTIFACTS_FNS),

    withDatabase(database: string): PensieveClient {
      // Wrap the transport so the default database is applied.
      // opts.database in the caller's per-request options wins because the spread
      // order is `{ database, ...opts }` — a defined opts.database comes last.
      const scoped: PensieveTransport = {
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

export function createPensieveClient(cfg: PensieveClientConfig): PensieveClient {
  return fromTransport(createTransport(cfg));
}
