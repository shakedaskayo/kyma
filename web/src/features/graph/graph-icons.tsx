import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  Brain,
  Server,
  FolderGit2,
  User,
  Table2,
  Columns3,
  Database,
  Network,
  FileText,
  Folder,
  FileCog,
  KeyRound,
  Lightbulb,
  Layers,
  Rocket,
  Box,
  Boxes,
  Webhook,
  Code2,
  Braces,
  Package,
  Activity,
  Eye,
  ListTree,
  FileJson,
  Cloud,
  Building2,
  GitPullRequest,
  CircleDot,
  Tag,
} from "lucide-react";
import {
  SiGithub,
  SiGitlab,
  SiBitbucket,
  SiNotion,
  SiConfluence,
  SiGoogledrive,
  SiGmail,
  SiJira,
  SiLinear,
  SiAsana,
  SiPostgresql,
  SiPrometheus,
  SiDatadog,
  SiKubernetes,
  SiDocker,
  SiGrafana,
  SiPagerduty,
  SiGooglecloud,
  SiSentry,
  SiOpentelemetry,
  SiRedis,
  SiMongodb,
  SiSnowflake,
  SiElastic,
  SiTerraform,
} from "@icons-pack/react-simple-icons";
import { SlackIcon, AmazonS3Icon } from "@/features/connectors/vendor-icons";
import { getLabelColor } from "@/sdk/graph-layout";

/**
 * Graph icon gallery — a single registry mapping entity kinds/labels and vendor
 * brands to icons, so the graph (and, once the backend exposes it, the agent)
 * can show "what each node is" at a glance.
 *
 * Two layers:
 *  • KIND_ICONS  — entity kind / node-label → lucide glyph (memory, service,
 *    person, table, infra, …).
 *  • BRAND_ICONS — vendor slug → brand mark (github, slack, datadog, k8s, …),
 *    reusing the same simple-icons set the connectors catalog uses.
 *
 * Resolution order for a node: explicit `properties.icon` → vendor hint
 * (source/vendor/brand/provider or a brand keyword in the label/name) → kind →
 * normalized label → fallback. Extend at runtime with `registerGraphIcons(...)`
 * (the hook a future settings/server-config "gallery" plugs into).
 */

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Icon = ComponentType<any>;

const KIND_ICONS: Record<string, Icon> = {
  memory: Brain,
  fact: Brain,
  decision: Lightbulb,
  preference: Tag,
  learning: Lightbulb,
  procedure: ListTree,
  service: Server,
  repo: FolderGit2,
  repository: FolderGit2,
  person: User,
  user: User,
  table: Table2,
  databasetable: Table2,
  column: Columns3,
  databasecolumn: Columns3,
  database: Database,
  schema: Network,
  file: FileText,
  codefile: FileText,
  directory: Folder,
  config: FileCog,
  configmap: FileCog,
  secret: KeyRound,
  credential: KeyRound,
  concept: Lightbulb,
  namespace: Layers,
  deployment: Rocket,
  pod: Box,
  infra: Boxes,
  infracomponent: Boxes,
  server: Server,
  endpoint: Webhook,
  apiendpoint: Webhook,
  api: Webhook,
  openapispec: FileJson,
  function: Code2,
  codefunction: Code2,
  class: Braces,
  codeclass: Braces,
  module: Package,
  trace: Activity,
  view: Eye,
  databaseview: Eye,
  index: ListTree,
  databaseindex: ListTree,
  pullrequest: GitPullRequest,
  issue: CircleDot,
  organization: Building2,
  label: Tag,
};

const BRAND_ICONS: Record<string, Icon> = {
  github: SiGithub,
  gitlab: SiGitlab,
  bitbucket: SiBitbucket,
  notion: SiNotion,
  confluence: SiConfluence,
  googledrive: SiGoogledrive,
  gmail: SiGmail,
  jira: SiJira,
  linear: SiLinear,
  asana: SiAsana,
  postgresql: SiPostgresql,
  postgres: SiPostgresql,
  prometheus: SiPrometheus,
  datadog: SiDatadog,
  kubernetes: SiKubernetes,
  k8s: SiKubernetes,
  docker: SiDocker,
  grafana: SiGrafana,
  pagerduty: SiPagerduty,
  gcp: SiGooglecloud,
  googlecloud: SiGooglecloud,
  sentry: SiSentry,
  opentelemetry: SiOpentelemetry,
  otel: SiOpentelemetry,
  redis: SiRedis,
  mongodb: SiMongodb,
  snowflake: SiSnowflake,
  elastic: SiElastic,
  elasticsearch: SiElastic,
  terraform: SiTerraform,
  slack: SlackIcon,
  amazons3: AmazonS3Icon,
  s3: AmazonS3Icon,
  aws: Cloud,
  amazon: Cloud,
};

/** Merge custom mappings (e.g. from server config / settings) at runtime. */
export function registerGraphIcons(opts: { kinds?: Record<string, Icon>; brands?: Record<string, Icon> }) {
  Object.assign(KIND_ICONS, opts.kinds ?? {});
  Object.assign(BRAND_ICONS, opts.brands ?? {});
}

/** All built-in keys, for a settings "gallery" preview. */
export function iconGallery() {
  return { kinds: Object.keys(KIND_ICONS), brands: Object.keys(BRAND_ICONS) };
}

const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");

/**
 * Connector-ingested nodes pack their extras (incl. `vendor`/`type`) into a
 * `props` JSON string. Merge that blob in so the resolvers see those keys as if
 * they were discrete properties. Discrete props win over the blob.
 */
function effectiveProps(p: Record<string, unknown>): Record<string, unknown> {
  const blob = p.props ?? p.properties ?? p.data;
  if (typeof blob === "string" && blob.trim().startsWith("{")) {
    try {
      return { ...(JSON.parse(blob) as Record<string, unknown>), ...p };
    } catch {
      /* not JSON — ignore */
    }
  }
  return p;
}

export interface ResolvedIcon {
  /** Stable cache key. */
  id: string;
  Comp: Icon;
  /** True for vendor brand marks (drawn slightly larger / brand-aware). */
  brand: boolean;
}

function findBrandKeyword(text: string): string | null {
  const t = norm(text);
  for (const key of Object.keys(BRAND_ICONS)) {
    if (t.includes(key)) return key;
  }
  return null;
}

// ── Classification taxonomy ───────────────────────────────────────────────
// Hierarchical `provider::resource` types (e.g. "kubernetes::pod",
// "aws::ec2::instance", "gcp::compute::instance"). The provider segment drives
// the brand mark; the resource segment drives the kind glyph + color. Arbitrary
// types resolve generically via this split, so connectors/the agent can mint any
// `provider::resource` without it being enumerated here.

/** Common cloud/infra resource → an existing kind key (for glyph + color). */
const RESOURCE_KIND: Record<string, string> = {
  pod: "pod",
  container: "pod",
  deployment: "deployment",
  replicaset: "deployment",
  statefulset: "deployment",
  daemonset: "deployment",
  service: "service",
  svc: "service",
  ingress: "endpoint",
  node: "infra",
  cluster: "infra",
  namespace: "namespace",
  configmap: "config",
  secret: "secret",
  instance: "service",
  vm: "service",
  ec2: "service",
  lambda: "function",
  function: "function",
  bucket: "database",
  s3: "database",
  volume: "database",
  disk: "database",
  database: "database",
  db: "database",
  rds: "database",
  table: "table",
  queue: "module",
  topic: "module",
  stream: "module",
  repo: "repo",
  repository: "repo",
  monitor: "service",
  dashboard: "concept",
  alert: "issue",
};

/** Pull a `type` hint from real props or flattened content. */
function getType(p: Record<string, unknown>): string {
  const str = (v: unknown) => (typeof v === "string" ? v : "");
  const direct = str(p.type) || str(p.resource_type) || str(p.resourcetype);
  if (direct) return direct;
  const m = str(p.content).match(/(?:^|\n)\s*type:\s*(.+)/i);
  return m ? m[1].trim() : "";
}

/** Split a `provider::resource` (or `/`, `:`) type into its ends. */
function splitType(type: string): { provider: string; resource: string } {
  const parts = type
    .split(/::|\/|:/)
    .map((s) => s.trim())
    .filter(Boolean);
  return {
    provider: parts[0] ?? "",
    resource: parts[parts.length - 1] ?? "",
  };
}

/**
 * Resolve an icon for a node from its labels + properties. Returns null when
 * nothing matches (caller falls back to a plain dot).
 */
export function resolveGraphIcon(
  labels: string[],
  properties: Record<string, unknown> | undefined,
): ResolvedIcon | null {
  const p = effectiveProps(properties ?? {});
  const str = (v: unknown) => (typeof v === "string" ? v : "");

  // Entity props (kind/vendor/source/icon) are flattened into `content` as
  // "Key: value" lines — parse those so they're resolvable like real props.
  const fromContent: Record<string, string> = {};
  for (const line of str(p.content).split("\n")) {
    const m = line.match(/^\s*([a-zA-Z_]+)\s*:\s*(.+?)\s*$/);
    if (m) fromContent[m[1].toLowerCase()] = m[2].trim();
  }
  const pick = (k: string) => norm(str(p[k]) || fromContent[k] || "");

  const brand = (key: string): ResolvedIcon => ({ id: `b:${key}`, Comp: BRAND_ICONS[key], brand: true });
  const kindIcon = (key: string): ResolvedIcon => ({ id: `k:${key}`, Comp: KIND_ICONS[key], brand: false });

  // 1. Explicit icon set by the backend / agent.
  const explicit = pick("icon");
  if (explicit && BRAND_ICONS[explicit]) return brand(explicit);
  if (explicit && KIND_ICONS[explicit]) return kindIcon(explicit);

  // 1.5 Classification type `provider::resource` (e.g. kubernetes::pod,
  // aws::ec2::instance): the provider drives the brand mark; failing that the
  // resource drives a kind glyph.
  const type = getType(p);
  if (type) {
    const { provider, resource } = splitType(type);
    const pv = norm(provider);
    if (pv && BRAND_ICONS[pv]) return brand(pv);
    const rk = norm(RESOURCE_KIND[norm(resource)] ?? resource);
    if (rk && KIND_ICONS[rk]) return kindIcon(rk);
  }

  // 2. Vendor hint from provenance keys (real props or flattened content).
  for (const k of ["vendor", "brand", "source_type", "sourcetype", "source", "provider", "connector"]) {
    const v = pick(k);
    if (v && BRAND_ICONS[v]) return brand(v);
  }

  // 3. Brand keyword in the label / name / title (not free-text content).
  const kw = findBrandKeyword(`${labels[0] ?? ""} ${str(p.name)} ${str(p.title)} ${str(p.full_name)}`);
  if (kw) return brand(kw);

  // 4. Entity kind (real prop or flattened "Kind: …"), or a memory's type
  // (fact / decision / preference / …) for plain memory nodes.
  const kind = pick("kind") || pick("memory_type");
  if (kind && KIND_ICONS[kind]) return kindIcon(kind);

  // 5. Normalized label (strip a leading "Database" prefix: DatabaseTable→table).
  const lbl = norm((labels[0] ?? "").replace(/^Database/, ""));
  if (lbl && KIND_ICONS[lbl]) return kindIcon(lbl);
  const rawLbl = norm(labels[0] ?? "");
  if (rawLbl && KIND_ICONS[rawLbl]) return kindIcon(rawLbl);

  return null;
}

// ── Per-kind node color ─────────────────────────────────────────────────────
// Distinct, palette-aligned hue per entity kind so the graph isn't a field of
// one color. Entities all carry the `Memory` label, so color must come from the
// *kind* (parsed from props/content); schema/connector nodes keep their
// label color.
const KIND_COLOR: Record<string, string> = {
  service: "#38bdf8", // sky
  server: "#38bdf8",
  repo: "#a78bfa", // violet
  repository: "#a78bfa",
  person: "#fb7185", // rose
  user: "#fb7185",
  team: "#fb7185",
  table: "#34d399", // emerald
  column: "#6ee7b7",
  database: "#22d3ee", // cyan
  schema: "#22d3ee",
  file: "#94a3b8", // slate
  directory: "#cbd5e1",
  config: "#818cf8", // indigo
  secret: "#f59e0b", // amber
  credential: "#f59e0b",
  concept: "#fbbf24", // yellow
  decision: "#c084fc",
  fact: "#38bdf8",
  preference: "#fbbf24",
  learning: "#34d399",
  procedure: "#2dd4bf",
  namespace: "#06b6d4",
  deployment: "#10b981",
  pod: "#2dd4bf", // teal
  infra: "#2dd4bf",
  endpoint: "#c084fc",
  api: "#c084fc",
  function: "#60a5fa",
  class: "#f472b6",
  module: "#a3e635",
  memory: "#fbbf24",
  issue: "#f87171",
  pullrequest: "#34d399",
  organization: "#fb7185",
};

/** Resolve a node's fill color: per-kind for entities, label color otherwise. */
export function resolveNodeColor(
  labels: string[],
  properties: Record<string, unknown> | undefined,
): string {
  const p = effectiveProps(properties ?? {});
  const str = (v: unknown) => (typeof v === "string" ? v : "");
  // Classification type wins: color by the resource segment (pod/instance/…).
  const type = getType(p);
  if (type) {
    const { resource } = splitType(type);
    const rk = norm(RESOURCE_KIND[norm(resource)] ?? resource);
    if (rk && KIND_COLOR[rk]) return KIND_COLOR[rk];
    if (resource) return getLabelColor(resource); // distinct hue per resource
  }
  let kind = norm(str(p.kind));
  if (!kind) {
    const m = str(p.content).match(/(?:^|\n)\s*kind:\s*([a-zA-Z ]+)/i);
    if (m) kind = norm(m[1]);
  }
  if (!kind) kind = norm(str(p.memory_type)); // fact / decision / preference …
  if (kind && KIND_COLOR[kind]) return KIND_COLOR[kind];
  if (kind) return getLabelColor(kind); // stable, distinct hue per unknown kind
  return getLabelColor(labels[0] ?? "");
}

// ── Canvas image cache ──────────────────────────────────────────────────────
// react-force-graph draws on a 2D canvas, so React icon components are rendered
// once to an <svg> data-URL, decoded to an <Image>, and cached for drawImage.
const imgCache = new Map<string, HTMLImageElement>();

/**
 * Get a cached HTMLImageElement for an icon at a given color. Returns the image
 * immediately (may not be loaded yet); `onReady` fires once it decodes so the
 * canvas can repaint.
 */
export function getIconImage(
  icon: ResolvedIcon,
  color: string,
  onReady: () => void,
): HTMLImageElement {
  const key = `${icon.id}:${color}`;
  const hit = imgCache.get(key);
  if (hit) return hit;
  // Render at 2× (40px) so the glyph stays crisp when drawn small; brand marks
  // get a slightly thinner stroke for legibility.
  const svg = renderToStaticMarkup(
    createElement(icon.Comp, { size: 40, color, stroke: color, strokeWidth: icon.brand ? 1.8 : 2 }),
  );
  const img = new Image(40, 40);
  img.onload = onReady;
  img.src = `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
  imgCache.set(key, img);
  return img;
}
