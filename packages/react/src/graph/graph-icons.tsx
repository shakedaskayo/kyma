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
  Workflow,
  SquareTerminal,
  ScrollText,
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
import { SlackIcon, AmazonS3Icon } from "../internal/vendor-icons";
import { getLabelColor } from "@kyma-ai/client";

/**
 * Graph icon gallery — a single registry mapping entity kinds/labels and vendor
 * brands to icons.
 * Adapted from web/src/features/graph/graph-icons.tsx (using internal imports).
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
  // CI / GitHub Actions. github::* nodes usually render the GitHub brand chip;
  // these are the kind-glyph fallback + drive the per-kind color below.
  workflowrun: Workflow,
  workflow: Workflow,
  job: SquareTerminal,
  logfile: ScrollText,
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

function getType(p: Record<string, unknown>): string {
  const str = (v: unknown) => (typeof v === "string" ? v : "");
  const direct = str(p.type) || str(p.resource_type) || str(p.resourcetype);
  if (direct) return direct;
  const m = str(p.content).match(/(?:^|\n)\s*type:\s*(.+)/i);
  return m ? m[1].trim() : "";
}

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

  // 1.5 Classification type `provider::resource`
  const type = getType(p);
  if (type) {
    const { provider, resource } = splitType(type);
    const pv = norm(provider);
    if (pv && BRAND_ICONS[pv]) return brand(pv);
    const rk = norm(RESOURCE_KIND[norm(resource)] ?? resource);
    if (rk && KIND_ICONS[rk]) return kindIcon(rk);
  }

  // 2. Vendor hint from provenance keys
  for (const k of ["vendor", "brand", "source_type", "sourcetype", "source", "provider", "connector"]) {
    const v = pick(k);
    if (v && BRAND_ICONS[v]) return brand(v);
  }

  // 3. Brand keyword in the label / name / title (not free-text content).
  const kw = findBrandKeyword(`${labels[0] ?? ""} ${str(p.name)} ${str(p.title)} ${str(p.full_name)}`);
  if (kw) return brand(kw);

  // 4. Entity kind (real prop or flattened "Kind: …")
  const kind = pick("kind") || pick("memory_type");
  if (kind && KIND_ICONS[kind]) return kindIcon(kind);

  // 5. Normalized label
  const lbl = norm((labels[0] ?? "").replace(/^Database/, ""));
  if (lbl && KIND_ICONS[lbl]) return kindIcon(lbl);
  const rawLbl = norm(labels[0] ?? "");
  if (rawLbl && KIND_ICONS[rawLbl]) return kindIcon(rawLbl);

  return null;
}

// ── Per-kind node color ─────────────────────────────────────────────────────
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
  // CI / GitHub Actions.
  workflowrun: "#fb923c", // orange — a pipeline run
  workflow: "#fb923c",
  job: "#f97316", // deeper orange — a job within a run
  logfile: "#94a3b8", // slate — a log artifact (like a file)
};

/** Resolve a node's fill color: per-kind for entities, label color otherwise. */
export function resolveNodeColor(
  labels: string[],
  properties: Record<string, unknown> | undefined,
): string {
  const p = effectiveProps(properties ?? {});
  const str = (v: unknown) => (typeof v === "string" ? v : "");
  const type = getType(p);
  if (type) {
    const { resource } = splitType(type);
    const rk = norm(RESOURCE_KIND[norm(resource)] ?? resource);
    if (rk && KIND_COLOR[rk]) return KIND_COLOR[rk];
    if (resource) return getLabelColor(resource);
  }
  let kind = norm(str(p.kind));
  if (!kind) {
    const m = str(p.content).match(/(?:^|\n)\s*kind:\s*([a-zA-Z ]+)/i);
    if (m) kind = norm(m[1]);
  }
  if (!kind) kind = norm(str(p.memory_type));
  if (kind && KIND_COLOR[kind]) return KIND_COLOR[kind];
  if (kind) return getLabelColor(kind);
  return getLabelColor(labels[0] ?? "");
}

// ── Canvas image cache ──────────────────────────────────────────────────────
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
  const svg = renderToStaticMarkup(
    createElement(icon.Comp, { size: 40, color, stroke: color, strokeWidth: icon.brand ? 1.8 : 2 }),
  );
  const img = new Image(40, 40);
  img.onload = onReady;
  img.src = `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
  imgCache.set(key, img);
  return img;
}
