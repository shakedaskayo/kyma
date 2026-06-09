import { useMemo, useState, type ComponentType } from "react";
import {
  ChevronDown,
  Layers,
  Search,
  X,
  Eye,
  EyeOff,
  Tag as TagIcon,
  ArrowRight,
  ArrowLeft,
  Copy,
  Sparkles,
  Sun,
  Waypoints,
  Spline,
  Circle as CircleIcon,
  LayoutGrid,
  ScrollText,
} from "lucide-react";
import {
  getLabelColor,
  type LayoutAlgorithm,
} from "@kyma-ai/client";
import { getRelationshipFamilyColor } from "./graph-style";
import { resolveGraphIcon } from "./graph-icons";
import type {
  ArtifactWindow,
  GraphNode,
  GraphRelationship,
  GraphStats,
} from "@kyma-ai/client";
import { useGraphStore } from "./graph-store";
import type { GraphCoord } from "../hooks/useKymaGraph";
import { graphKey } from "../hooks/useKymaGraph";
import { useKymaClient } from "../provider/context";
import { cn } from "../internal/cn";

/**
 * Single right-hand sidebar that owns every off-canvas control: graph
 * picker, search, selected-node inspector, and the legend
 * (namespaces / node types / relationships). All colours come from the
 * shadcn theme tokens (`bg-card`, `text-muted-foreground`, etc.) so the
 * graph view participates in the app's light/dark mode like every other
 * surface — no hard-coded hexes.
 */

const LAYOUTS: { value: LayoutAlgorithm; label: string }[] = [
  { value: "force", label: "Force" },
  { value: "tree", label: "Tree" },
  { value: "radial", label: "Radial" },
  { value: "grid", label: "Grid" },
];

export interface GraphSidebarProps {
  coords: GraphCoord[];
  namespaceCounts: Record<string, number> | undefined;
  stats: GraphStats | undefined;
  visibleNodes: number;
  visibleEdges: number;
  totalNodes: number;
  totalEdges: number;
  /** Progress text shown while per-namespace fetches are still in flight. */
  loadingText: string | null;
  selectedNode: GraphNode | null;
  nodesByCompositeId: Map<string, GraphNode>;
  edges: GraphRelationship[];
  onSelectComposite: (id: string | null) => void;
  onExpand: () => void;
}

export function GraphSidebar(props: GraphSidebarProps) {
  const graph = useGraphStore((s) => s.graph);
  const setGraph = useGraphStore((s) => s.setGraph);
  const layout = useGraphStore((s) => s.layout);
  const setLayout = useGraphStore((s) => s.setLayout);
  const showEdgeLabels = useGraphStore((s) => s.showEdgeLabels);
  const toggleEdgeLabels = useGraphStore((s) => s.toggleEdgeLabels);
  const glow = useGraphStore((s) => s.glow);
  const toggleGlow = useGraphStore((s) => s.toggleGlow);
  const animatedFlow = useGraphStore((s) => s.animatedFlow);
  const toggleAnimatedFlow = useGraphStore((s) => s.toggleAnimatedFlow);
  const curvedEdges = useGraphStore((s) => s.curvedEdges);
  const toggleCurvedEdges = useGraphStore((s) => s.toggleCurvedEdges);
  const communityHulls = useGraphStore((s) => s.communityHulls);
  const toggleCommunityHulls = useGraphStore((s) => s.toggleCommunityHulls);
  const sizeByDegree = useGraphStore((s) => s.sizeByDegree);
  const toggleSizeByDegree = useGraphStore((s) => s.toggleSizeByDegree);
  const searchQuery = useGraphStore((s) => s.searchQuery);
  const setSearchQuery = useGraphStore((s) => s.setSearchQuery);
  const hiddenLabels = useGraphStore((s) => s.hiddenLabels);
  const toggleHiddenLabel = useGraphStore((s) => s.toggleHiddenLabel);
  const hiddenNamespaces = useGraphStore((s) => s.hiddenNamespaces);
  const toggleHiddenNamespace = useGraphStore((s) => s.toggleHiddenNamespace);
  const setHiddenNamespaces = useGraphStore((s) => s.setHiddenNamespaces);
  const relTypeFilter = useGraphStore((s) => s.relTypeFilter);
  const setRelTypeFilter = useGraphStore((s) => s.setRelTypeFilter);

  const namespaceEntries = useMemo(() => {
    if (!props.namespaceCounts) return [] as [string, number][];
    return Object.entries(props.namespaceCounts).sort(([, a], [, b]) => b - a);
  }, [props.namespaceCounts]);

  const labelEntries = useMemo(() => {
    if (!props.stats) return [] as [string, number][];
    return Object.entries(props.stats.label_counts).sort(([, a], [, b]) => b - a);
  }, [props.stats]);

  const relEntries = useMemo(() => {
    if (!props.stats) return [] as [string, number][];
    return Object.entries(props.stats.relationship_type_counts).sort(
      ([, a], [, b]) => b - a,
    );
  }, [props.stats]);

  const showNamespaceFilter = graph === "all" && namespaceEntries.length > 1;
  const noNamespaceHidden = hiddenNamespaces.length === 0;

  return (
    <aside className="ky-flex ky-h-full ky-w-80 ky-flex-col ky-border-l ky-bg-card ky-text-card-foreground">
      {/* Focus picker */}
      <div className="ky-shrink-0 ky-border-b ky-p-3">
        <label className="ky-mb-1.5 ky-block ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
          Focus
        </label>
        <div className="ky-relative">
          <select
            value={graph}
            onChange={(e) => setGraph(e.target.value)}
            className="ky-w-full ky-appearance-none ky-rounded-md ky-border ky-bg-background ky-px-3 ky-py-2 ky-pr-8 ky-text-xs ky-outline-none ky-transition-colors focus:ky-border-primary"
            title="Graph namespace"
          >
            <option value="all">All databases & graphs</option>
            {props.coords.map((c) => {
              const key = graphKey(c.database, c.graph);
              return (
                <option key={key} value={key}>
                  {c.database} / {c.graph}
                  {c.kind === "stored" ? " (stored)" : ""}
                </option>
              );
            })}
          </select>
          <ChevronDown className="ky-pointer-events-none ky-absolute ky-right-2 ky-top-1/2 ky-h-3.5 ky-w-3.5 -ky-translate-y-1/2 ky-text-muted-foreground" />
        </div>
      </div>

      {/* Search */}
      <div className="ky-shrink-0 ky-border-b ky-p-3">
        <div className="ky-relative">
          <Search className="ky-pointer-events-none ky-absolute ky-left-2.5 ky-top-1/2 ky-h-3.5 ky-w-3.5 -ky-translate-y-1/2 ky-text-muted-foreground" />
          <input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search nodes…"
            className="ky-w-full ky-rounded-md ky-border ky-bg-background ky-py-2 ky-pl-8 ky-pr-7 ky-text-xs ky-outline-none ky-transition-colors placeholder:ky-text-muted-foreground focus:ky-border-primary"
          />
          {searchQuery && (
            <button
              type="button"
              onClick={() => setSearchQuery("")}
              className="ky-absolute ky-right-1.5 ky-top-1/2 -ky-translate-y-1/2 ky-rounded ky-p-0.5 ky-text-muted-foreground hover:ky-bg-muted hover:ky-text-foreground"
              title="Clear search"
            >
              <X className="ky-h-3 ky-w-3" />
            </button>
          )}
        </div>
      </div>

      {/* Inspector / Legend scroll area */}
      <div className="ky-min-h-0 ky-flex-1 ky-overflow-y-auto">
        {props.selectedNode ? (
          <NodeInspector
            node={props.selectedNode}
            edges={props.edges}
            nodesByCompositeId={props.nodesByCompositeId}
            onSelectComposite={props.onSelectComposite}
            onExpand={props.onExpand}
          />
        ) : (
          <div className="ky-flex ky-flex-col ky-items-center ky-justify-center ky-gap-2 ky-p-6 ky-text-center ky-text-muted-foreground">
            <Sparkles className="ky-h-5 ky-w-5" />
            <p className="ky-text-xs">Click a node to inspect it</p>
          </div>
        )}

        <div className="ky-space-y-4 ky-border-t ky-p-3">
          {showNamespaceFilter && (
            <SectionGraphs
              entries={namespaceEntries}
              hidden={hiddenNamespaces}
              onToggle={toggleHiddenNamespace}
              noneHidden={noNamespaceHidden}
              onSelectAll={() => setHiddenNamespaces([])}
              onSelectNone={() =>
                setHiddenNamespaces(namespaceEntries.map(([ns]) => ns))
              }
            />
          )}

          <SectionLabels
            entries={labelEntries}
            hidden={hiddenLabels}
            onToggle={toggleHiddenLabel}
          />

          <SectionRels
            entries={relEntries}
            active={relTypeFilter}
            onSetActive={(r) => setRelTypeFilter(r === relTypeFilter ? null : r)}
          />
        </div>
      </div>

      {/* Footer */}
      <div className="ky-shrink-0 ky-border-t ky-bg-muted/30 ky-p-3">
        <div className="ky-mb-2 ky-flex ky-items-center ky-gap-1">
          {LAYOUTS.map((l) => (
            <button
              key={l.value}
              type="button"
              onClick={() => setLayout(l.value)}
              className={cn(
                "ky-flex-1 ky-rounded ky-px-2 ky-py-1 ky-text-[10px] ky-font-medium ky-transition-colors",
                layout === l.value
                  ? "ky-bg-primary ky-text-primary-foreground"
                  : "ky-border ky-border-transparent ky-text-muted-foreground hover:ky-border-border hover:ky-text-foreground",
              )}
            >
              {l.label}
            </button>
          ))}
        </div>
        <div className="ky-mb-2 ky-grid ky-grid-cols-3 ky-gap-1">
          <StyleToggle icon={Sun} label="Glow" on={glow} onClick={toggleGlow} title="Glow halos on focus/landmark nodes" />
          <StyleToggle icon={Waypoints} label="Flow" on={animatedFlow} onClick={toggleAnimatedFlow} title="Animated flow on the focused edges" />
          <StyleToggle icon={Spline} label="Curve" on={curvedEdges} onClick={toggleCurvedEdges} title="Curve edges" />
          <StyleToggle icon={LayoutGrid} label="Clusters" on={communityHulls} onClick={toggleCommunityHulls} title="Translucent community hulls" />
          <StyleToggle icon={CircleIcon} label="Size" on={sizeByDegree} onClick={toggleSizeByDegree} title="Size nodes by degree" />
          <StyleToggle icon={TagIcon} label="Labels" on={showEdgeLabels} onClick={toggleEdgeLabels} title="Relationship labels on edges" />
        </div>
        <div className="ky-flex ky-items-center ky-justify-between ky-text-[10px] ky-text-muted-foreground">
          <span>
            {props.visibleNodes === props.totalNodes
              ? `${props.totalNodes.toLocaleString()} nodes`
              : `${props.visibleNodes.toLocaleString()} / ${props.totalNodes.toLocaleString()} nodes`}
            {" · "}
            {props.visibleEdges.toLocaleString()} edges
          </span>
          {props.loadingText && (
            <span className="ky-font-mono ky-text-amber-500 dark:ky-text-amber-300">
              {props.loadingText}
            </span>
          )}
        </div>
      </div>
    </aside>
  );
}

/* ─── Sections ──────────────────────────────────────────────────────────── */

function SectionGraphs({
  entries,
  hidden,
  onToggle,
  noneHidden,
  onSelectAll,
  onSelectNone,
}: {
  entries: [string, number][];
  hidden: string[];
  onToggle: (ns: string) => void;
  noneHidden: boolean;
  onSelectAll: () => void;
  onSelectNone: () => void;
}) {
  if (entries.length === 0) return null;
  return (
    <div>
      <div className="ky-mb-1.5 ky-flex ky-items-center ky-justify-between">
        <span className="ky-flex ky-items-center ky-gap-1.5 ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
          <Layers className="ky-h-3 ky-w-3" /> Graphs
        </span>
        <button
          type="button"
          onClick={noneHidden ? onSelectNone : onSelectAll}
          className="ky-text-[10px] ky-font-medium ky-text-primary ky-transition-opacity hover:ky-opacity-80"
          title={noneHidden ? "Hide all" : "Show all"}
        >
          {noneHidden ? "Hide all" : "Show all"}
        </button>
      </div>
      <div className="ky-space-y-px">
        {entries.map(([ns, count]) => {
          const isHidden = hidden.includes(ns);
          const color = getLabelColor(ns);
          return (
            <button
              key={ns}
              type="button"
              onClick={() => onToggle(ns)}
              className={cn(
                "ky-group ky-flex ky-w-full ky-items-center ky-gap-2 ky-rounded ky-px-1.5 ky-py-1 ky-text-left ky-text-xs ky-transition-colors",
                isHidden
                  ? "ky-text-muted-foreground/60"
                  : "hover:ky-bg-accent/60 hover:ky-text-accent-foreground",
              )}
              title={isHidden ? `Show ${ns}` : `Hide ${ns}`}
            >
              <Checkbox checked={!isHidden} color={color} />
              <span className={cn("ky-flex-1 ky-truncate", isHidden && "ky-line-through")}>
                {ns}
              </span>
              <span className="ky-font-mono ky-text-[10px] ky-tabular-nums ky-text-muted-foreground">
                {count.toLocaleString()}
              </span>
              {isHidden ? (
                <EyeOff className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground/70" />
              ) : (
                <Eye className="ky-h-3 ky-w-3 ky-shrink-0 ky-opacity-0 ky-transition-opacity group-hover:ky-opacity-50" />
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function SectionLabels({
  entries,
  hidden,
  onToggle,
}: {
  entries: [string, number][];
  hidden: string[];
  onToggle: (label: string) => void;
}) {
  if (entries.length === 0) return null;
  return (
    <div>
      <span className="ky-mb-1.5 ky-block ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
        Node types
      </span>
      <div className="ky-space-y-px">
        {entries.map(([label, count]) => {
          const isHidden = hidden.includes(label);
          const color = getLabelColor(label);
          const ic = resolveGraphIcon([label], {});
          return (
            <button
              key={label}
              type="button"
              onClick={() => onToggle(label)}
              className={cn(
                "ky-group ky-flex ky-w-full ky-items-center ky-gap-2 ky-rounded ky-px-1.5 ky-py-1 ky-text-left ky-text-xs ky-transition-colors",
                isHidden
                  ? "ky-text-muted-foreground/60"
                  : "hover:ky-bg-accent/60 hover:ky-text-accent-foreground",
              )}
              title={isHidden ? `Show ${label}` : `Hide ${label}`}
            >
              {ic ? (
                <ic.Comp size={14} color={color} className={cn("ky-shrink-0", isHidden && "ky-opacity-40")} />
              ) : (
                <Dot color={color} dim={isHidden} />
              )}
              <span className={cn("ky-flex-1 ky-truncate", isHidden && "ky-line-through")}>
                {label}
              </span>
              <span className="ky-font-mono ky-text-[10px] ky-tabular-nums ky-text-muted-foreground">
                {count.toLocaleString()}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function SectionRels({
  entries,
  active,
  onSetActive,
}: {
  entries: [string, number][];
  active: string | null;
  onSetActive: (r: string) => void;
}) {
  if (entries.length === 0) return null;
  return (
    <div>
      <span className="ky-mb-1.5 ky-block ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
        Relationships
      </span>
      <div className="ky-space-y-px">
        {entries.map(([rel, count]) => {
          const color = getRelationshipFamilyColor(rel);
          const isActive = active === rel;
          return (
            <button
              key={rel}
              type="button"
              onClick={() => onSetActive(rel)}
              className={cn(
                "ky-flex ky-w-full ky-items-center ky-gap-2 ky-rounded ky-px-1.5 ky-py-1 ky-text-left ky-text-xs ky-transition-colors",
                isActive
                  ? "ky-bg-accent ky-text-accent-foreground"
                  : "ky-text-muted-foreground hover:ky-bg-accent/60 hover:ky-text-foreground",
              )}
            >
              <span
                className="ky-h-[3px] ky-w-3.5 ky-shrink-0 ky-rounded-full"
                style={{ backgroundColor: color }}
              />
              <span className="ky-flex-1 ky-truncate">{rel}</span>
              <span className="ky-font-mono ky-text-[10px] ky-tabular-nums ky-text-muted-foreground">
                {count.toLocaleString()}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

/* ─── Log-file viewer ───────────────────────────────────────────────────── */

/** Pull an artifact `object_path` from a node's properties — either a direct
 *  `object_path` key, or one nested inside a JSON-encoded `props` blob (how the
 *  stored-graph provider serialises node props). Returns null when absent. */
function objectPathOf(props: Record<string, unknown>): string | null {
  const direct = props.object_path;
  if (typeof direct === "string" && direct) return direct;
  const blob = props.props;
  if (typeof blob === "string" && blob.trim().startsWith("{")) {
    try {
      const parsed = JSON.parse(blob) as Record<string, unknown>;
      if (typeof parsed.object_path === "string") return parsed.object_path;
    } catch {
      /* not JSON — ignore */
    }
  }
  return null;
}

/** Inline viewer for a `LogFile` node (CI job logs, etc.): fetches the artifact
 *  from the object store by path and pages through it with "Load more". */
function LogFileViewer({ path }: { path: string }) {
  const client = useKymaClient();
  const [win, setWin] = useState<ArtifactWindow | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = async (offset: number) => {
    setLoading(true);
    try {
      const w = await client.artifacts.fetchArtifactByPath(path, { offset, limit: 65536 });
      setWin((prev) => (prev && offset > 0 ? { ...w, content: prev.content + w.content } : w));
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="ky-mb-3">
      {!win ? (
        <button
          type="button"
          onClick={() => void load(0)}
          disabled={loading}
          className="ky-flex ky-w-full ky-items-center ky-justify-center ky-gap-1.5 ky-rounded-md ky-border ky-bg-background ky-py-1.5 ky-text-[11px] ky-font-medium ky-transition-colors hover:ky-bg-accent disabled:ky-opacity-50"
        >
          <ScrollText className="ky-h-3.5 ky-w-3.5" /> {loading ? "Loading…" : "View log"}
        </button>
      ) : (
        <div className="ky-rounded-md ky-border">
          <div className="ky-flex ky-items-center ky-gap-2 ky-border-b ky-px-2 ky-py-1 ky-text-[10px] ky-text-muted-foreground">
            <ScrollText className="ky-h-3 ky-w-3" /> log
            <span className="ky-ml-auto ky-font-mono">{win.size_bytes.toLocaleString()} B</span>
          </div>
          <pre className="ky-max-h-72 ky-overflow-auto ky-whitespace-pre-wrap ky-break-all ky-p-2 ky-text-[10px] ky-leading-snug">
            {win.content || "(empty)"}
          </pre>
          {!win.eof && (
            <button
              type="button"
              onClick={() => void load(win.offset + win.returned_bytes)}
              disabled={loading}
              className="ky-w-full ky-border-t ky-py-1 ky-text-[10px] ky-text-muted-foreground ky-transition-colors hover:ky-bg-accent disabled:ky-opacity-50"
            >
              {loading ? "Loading…" : "Load more"}
            </button>
          )}
        </div>
      )}
      {error && <p className="ky-mt-1 ky-text-[10px] ky-text-destructive">{error}</p>}
    </div>
  );
}

/* ─── Inspector ─────────────────────────────────────────────────────────── */

function NodeInspector({
  node,
  edges,
  nodesByCompositeId,
  onSelectComposite,
  onExpand,
}: {
  node: GraphNode;
  edges: GraphRelationship[];
  nodesByCompositeId: Map<string, GraphNode>;
  onSelectComposite: (id: string | null) => void;
  onExpand: () => void;
}) {
  const ns = node.namespace ?? "";
  const displayName = (node.properties.name as string | undefined) ?? node.id;
  const primaryLabel = node.labels[0] ?? "Node";
  const dotColor = getLabelColor(primaryLabel);

  // LogFile nodes (CI job logs, scraped files, etc.) carry an `object_path` —
  // surface an inline log viewer that streams the artifact from the object store.
  const logPath = useMemo(() => {
    if (!node.labels.some((l) => l.toLowerCase() === "logfile")) return null;
    return objectPathOf(node.properties);
  }, [node]);

  const neighbours = useMemo(() => {
    const out: {
      compositeId: string;
      name: string;
      relType: string;
      color: string;
      direction: "out" | "in";
      relId: string;
    }[] = [];
    for (const rel of edges) {
      if ((rel.namespace ?? "") !== ns) continue;
      let otherId: string | null = null;
      let direction: "out" | "in" = "out";
      if (rel.source_id === node.id) {
        otherId = rel.target_id;
        direction = "out";
      } else if (rel.target_id === node.id) {
        otherId = rel.source_id;
        direction = "in";
      }
      if (!otherId) continue;
      const compositeId = `${ns}::${otherId}`;
      const other = nodesByCompositeId.get(compositeId);
      const name = other
        ? ((other.properties.name as string | undefined) ?? otherId)
        : otherId;
      out.push({
        compositeId,
        name,
        relType: rel.relationship_type,
        color: getRelationshipFamilyColor(rel.relationship_type),
        direction,
        relId: rel.id,
      });
    }
    return out;
  }, [edges, ns, node.id, nodesByCompositeId]);

  return (
    <div className="ky-p-4">
      <div className="ky-mb-3 ky-flex ky-items-start ky-gap-2">
        <div
          className="ky-mt-0.5 ky-h-2.5 ky-w-2.5 ky-shrink-0 ky-rounded-full"
          style={{ backgroundColor: dotColor }}
        />
        <div className="ky-min-w-0 ky-flex-1">
          <p className="ky-truncate ky-text-sm ky-font-medium" title={displayName}>
            {displayName}
          </p>
          <p className="ky-mt-0.5 ky-truncate ky-text-[10px] ky-text-muted-foreground">
            {primaryLabel} · {neighbours.length} {neighbours.length === 1 ? "link" : "links"} · {ns}
          </p>
        </div>
        <button
          type="button"
          onClick={() => onSelectComposite(null)}
          className="ky-rounded ky-p-1 ky-text-muted-foreground ky-transition-colors hover:ky-bg-muted hover:ky-text-foreground"
          aria-label="Close inspector"
        >
          <X className="ky-h-3.5 ky-w-3.5" />
        </button>
      </div>

      <button
        type="button"
        onClick={onExpand}
        className="ky-mb-3 ky-w-full ky-rounded-md ky-border ky-bg-background ky-py-1.5 ky-text-[11px] ky-font-medium ky-transition-colors hover:ky-bg-accent hover:ky-text-accent-foreground"
      >
        Expand neighbours
      </button>

      {logPath && <LogFileViewer path={logPath} />}

      {/* Properties */}
      <div className="ky-mb-3">
        <p className="ky-mb-1 ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
          Properties
        </p>
        <div className="ky-space-y-1.5">
          {Object.entries(node.properties)
            .filter(([key]) => key !== "name")
            .map(([key, value]) => {
              const strValue =
                typeof value === "object" ? JSON.stringify(value) : String(value);
              const display =
                strValue.length > 90 ? strValue.slice(0, 90) + "…" : strValue;
              return (
                <div key={key}>
                  <p className="ky-text-[10px] ky-text-muted-foreground">{key}</p>
                  <button
                    type="button"
                    onClick={() => navigator.clipboard.writeText(strValue)}
                    className="ky-group ky-break-all ky-text-left ky-text-[11px] ky-transition-colors hover:ky-text-primary"
                    title="Copy"
                  >
                    {display}
                    <Copy className="ky-ml-1 ky-inline ky-h-2.5 ky-w-2.5 ky-opacity-0 group-hover:ky-opacity-60" />
                  </button>
                </div>
              );
            })}
        </div>
      </div>

      {/* Neighbours */}
      {neighbours.length > 0 && (
        <div>
          <p className="ky-mb-1 ky-flex ky-items-center ky-justify-between ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
            <span>Neighbours</span>
            <span className="ky-font-mono">{neighbours.length}</span>
          </p>
          <div className="ky-space-y-px">
            {neighbours.map((n) => (
              <button
                key={n.relId}
                type="button"
                onClick={() => onSelectComposite(n.compositeId)}
                className="ky-flex ky-w-full ky-items-center ky-gap-2 ky-rounded ky-border-l-2 ky-px-2 ky-py-1 ky-text-left ky-text-[11px] ky-transition-colors hover:ky-bg-accent/60"
                style={{ borderColor: n.color }}
                title={n.relType}
              >
                {n.direction === "out" ? (
                  <ArrowRight className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground" />
                ) : (
                  <ArrowLeft className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground" />
                )}
                <span className="ky-truncate">{n.name}</span>
                <span className="ky-ml-auto ky-truncate ky-font-mono ky-text-[9.5px] ky-text-muted-foreground">
                  {n.relType}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

/* ─── Small visuals ─────────────────────────────────────────────────────── */

function StyleToggle({
  icon: Icon,
  label,
  on,
  onClick,
  title,
}: {
  icon: ComponentType<{ className?: string }>;
  label: string;
  on: boolean;
  onClick: () => void;
  title: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      aria-pressed={on}
      className={cn(
        "ky-flex ky-items-center ky-justify-center ky-gap-1 ky-rounded ky-px-1.5 ky-py-1 ky-text-[10px] ky-font-medium ky-transition-colors",
        on
          ? "ky-bg-accent ky-text-accent-foreground"
          : "ky-border ky-text-muted-foreground hover:ky-text-foreground",
      )}
    >
      <Icon className="ky-h-3 ky-w-3" /> {label}
    </button>
  );
}

function Checkbox({ checked, color }: { checked: boolean; color: string }) {
  return (
    <span
      aria-hidden
      className="ky-relative ky-inline-block ky-h-3 ky-w-3 ky-shrink-0 ky-rounded-[3px] ky-border ky-transition-colors"
      style={{
        background: checked ? color : "transparent",
        borderColor: checked ? color : "hsl(var(--border))",
      }}
    >
      {checked && (
        <span
          className="ky-absolute"
          style={{
            left: 2.5,
            top: 0,
            width: 3,
            height: 6,
            border: "solid #fff",
            borderWidth: "0 1.5px 1.5px 0",
            transform: "rotate(45deg)",
          }}
        />
      )}
    </span>
  );
}

function Dot({ color, dim }: { color: string; dim: boolean }) {
  return (
    <span
      aria-hidden
      className="ky-h-2.5 ky-w-2.5 ky-shrink-0 ky-rounded-full ky-transition-opacity"
      style={{ backgroundColor: color, opacity: dim ? 0.3 : 1 }}
    />
  );
}
