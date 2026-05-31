import { useMemo } from "react";
import {
  ChevronDown,
  Layers,
  Search,
  X,
  Eye,
  EyeOff,
  Map as MapIcon,
  Tag as TagIcon,
  ArrowRight,
  ArrowLeft,
  Copy,
  Sparkles,
} from "lucide-react";
import {
  getLabelColor,
  getRelationshipColor,
  type LayoutAlgorithm,
} from "@/sdk/graph-layout";
import type { GraphNode, GraphRelationship, GraphStats } from "@/sdk/graph";
import { useGraphStore } from "./graph-store";
import type { GraphCoord } from "./useGraph";
import { graphKey } from "./useGraph";
import { cn } from "@/lib/utils";

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
  const showMiniMap = useGraphStore((s) => s.showMiniMap);
  const setShowMiniMap = useGraphStore((s) => s.setShowMiniMap);
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
    <aside className="flex h-full w-80 flex-col border-l bg-card text-card-foreground">
      {/* Focus picker */}
      <div className="shrink-0 border-b p-3">
        <label className="mb-1.5 block text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Focus
        </label>
        <div className="relative">
          <select
            value={graph}
            onChange={(e) => setGraph(e.target.value)}
            className="w-full appearance-none rounded-md border bg-background px-3 py-2 pr-8 text-xs outline-none transition-colors focus:border-primary"
            title="Graph namespace"
          >
            <option value="all">All databases & graphs</option>
            {props.coords.map((c) => {
              const key = graphKey(c);
              return (
                <option key={key} value={key}>
                  {c.database} / {c.graph}
                  {c.kind === "stored" ? " (stored)" : ""}
                </option>
              );
            })}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        </div>
      </div>

      {/* Search */}
      <div className="shrink-0 border-b p-3">
        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search nodes…"
            className="w-full rounded-md border bg-background py-2 pl-8 pr-7 text-xs outline-none transition-colors placeholder:text-muted-foreground focus:border-primary"
          />
          {searchQuery && (
            <button
              type="button"
              onClick={() => setSearchQuery("")}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
              title="Clear search"
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </div>
      </div>

      {/* Inspector / Legend scroll area */}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {props.selectedNode ? (
          <NodeInspector
            node={props.selectedNode}
            edges={props.edges}
            nodesByCompositeId={props.nodesByCompositeId}
            onSelectComposite={props.onSelectComposite}
            onExpand={props.onExpand}
          />
        ) : (
          <div className="flex flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
            <Sparkles className="h-5 w-5" />
            <p className="text-xs">Click a node to inspect it</p>
          </div>
        )}

        <div className="space-y-4 border-t p-3">
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
      <div className="shrink-0 border-t bg-muted/30 p-3">
        <div className="mb-2 flex items-center gap-1">
          {LAYOUTS.map((l) => (
            <button
              key={l.value}
              type="button"
              onClick={() => setLayout(l.value)}
              className={cn(
                "flex-1 rounded px-2 py-1 text-[10px] font-medium transition-colors",
                layout === l.value
                  ? "bg-primary text-primary-foreground"
                  : "border border-transparent text-muted-foreground hover:border-border hover:text-foreground",
              )}
            >
              {l.label}
            </button>
          ))}
        </div>
        <div className="mb-2 flex items-center gap-2">
          <button
            type="button"
            onClick={toggleEdgeLabels}
            className={cn(
              "flex flex-1 items-center justify-center gap-1.5 rounded px-2 py-1 text-[10px] font-medium transition-colors",
              showEdgeLabels
                ? "bg-accent text-accent-foreground"
                : "border text-muted-foreground hover:text-foreground",
            )}
            title="Show relationship labels on edges"
          >
            <TagIcon className="h-3 w-3" /> Labels
          </button>
          <button
            type="button"
            onClick={() => setShowMiniMap(!showMiniMap)}
            className={cn(
              "flex flex-1 items-center justify-center gap-1.5 rounded px-2 py-1 text-[10px] font-medium transition-colors",
              showMiniMap
                ? "bg-accent text-accent-foreground"
                : "border text-muted-foreground hover:text-foreground",
            )}
            title="Show minimap"
          >
            <MapIcon className="h-3 w-3" /> Minimap
          </button>
        </div>
        <div className="flex items-center justify-between text-[10px] text-muted-foreground">
          <span>
            {props.visibleNodes === props.totalNodes
              ? `${props.totalNodes.toLocaleString()} nodes`
              : `${props.visibleNodes.toLocaleString()} / ${props.totalNodes.toLocaleString()} nodes`}
            {" · "}
            {props.visibleEdges.toLocaleString()} edges
          </span>
          {props.loadingText && (
            <span className="font-mono text-amber-500 dark:text-amber-300">
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
      <div className="mb-1.5 flex items-center justify-between">
        <span className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          <Layers className="h-3 w-3" /> Graphs
        </span>
        <button
          type="button"
          onClick={noneHidden ? onSelectNone : onSelectAll}
          className="text-[10px] font-medium text-primary transition-opacity hover:opacity-80"
          title={noneHidden ? "Hide all" : "Show all"}
        >
          {noneHidden ? "Hide all" : "Show all"}
        </button>
      </div>
      <div className="space-y-px">
        {entries.map(([ns, count]) => {
          const isHidden = hidden.includes(ns);
          const color = getLabelColor(ns);
          return (
            <button
              key={ns}
              type="button"
              onClick={() => onToggle(ns)}
              className={cn(
                "group flex w-full items-center gap-2 rounded px-1.5 py-1 text-left text-xs transition-colors",
                isHidden
                  ? "text-muted-foreground/60"
                  : "hover:bg-accent/60 hover:text-accent-foreground",
              )}
              title={isHidden ? `Show ${ns}` : `Hide ${ns}`}
            >
              <Checkbox checked={!isHidden} color={color} />
              <span className={cn("flex-1 truncate", isHidden && "line-through")}>
                {ns}
              </span>
              <span className="font-mono text-[10px] tabular-nums text-muted-foreground">
                {count.toLocaleString()}
              </span>
              {isHidden ? (
                <EyeOff className="h-3 w-3 shrink-0 text-muted-foreground/70" />
              ) : (
                <Eye className="h-3 w-3 shrink-0 opacity-0 transition-opacity group-hover:opacity-50" />
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
      <span className="mb-1.5 block text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        Node types
      </span>
      <div className="space-y-px">
        {entries.map(([label, count]) => {
          const isHidden = hidden.includes(label);
          const color = getLabelColor(label);
          return (
            <button
              key={label}
              type="button"
              onClick={() => onToggle(label)}
              className={cn(
                "group flex w-full items-center gap-2 rounded px-1.5 py-1 text-left text-xs transition-colors",
                isHidden
                  ? "text-muted-foreground/60"
                  : "hover:bg-accent/60 hover:text-accent-foreground",
              )}
              title={isHidden ? `Show ${label}` : `Hide ${label}`}
            >
              <Dot color={color} dim={isHidden} />
              <span className={cn("flex-1 truncate", isHidden && "line-through")}>
                {label}
              </span>
              <span className="font-mono text-[10px] tabular-nums text-muted-foreground">
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
      <span className="mb-1.5 block text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        Relationships
      </span>
      <div className="space-y-px">
        {entries.map(([rel, count]) => {
          const color = getRelationshipColor(rel);
          const isActive = active === rel;
          return (
            <button
              key={rel}
              type="button"
              onClick={() => onSetActive(rel)}
              className={cn(
                "flex w-full items-center gap-2 rounded px-1.5 py-1 text-left text-xs transition-colors",
                isActive
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
              )}
            >
              <span
                className="h-[3px] w-3.5 shrink-0 rounded-full"
                style={{ backgroundColor: color }}
              />
              <span className="flex-1 truncate">{rel}</span>
              <span className="font-mono text-[10px] tabular-nums text-muted-foreground">
                {count.toLocaleString()}
              </span>
            </button>
          );
        })}
      </div>
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
        color: getRelationshipColor(rel.relationship_type),
        direction,
        relId: rel.id,
      });
    }
    return out;
  }, [edges, ns, node.id, nodesByCompositeId]);

  return (
    <div className="p-4">
      <div className="mb-3 flex items-start gap-2">
        <div
          className="mt-0.5 h-2.5 w-2.5 shrink-0 rounded-full"
          style={{ backgroundColor: dotColor }}
        />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium" title={displayName}>
            {displayName}
          </p>
          <p className="mt-0.5 text-[10px] text-muted-foreground">
            {primaryLabel} · {ns}
          </p>
        </div>
        <button
          type="button"
          onClick={() => onSelectComposite(null)}
          className="rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          aria-label="Close inspector"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      <button
        type="button"
        onClick={onExpand}
        className="mb-3 w-full rounded-md border bg-background py-1.5 text-[11px] font-medium transition-colors hover:bg-accent hover:text-accent-foreground"
      >
        Expand neighbours
      </button>

      {/* Properties */}
      <div className="mb-3">
        <p className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Properties
        </p>
        <div className="space-y-1.5">
          {Object.entries(node.properties)
            .filter(([key]) => key !== "name")
            .map(([key, value]) => {
              const strValue =
                typeof value === "object" ? JSON.stringify(value) : String(value);
              const display =
                strValue.length > 90 ? strValue.slice(0, 90) + "…" : strValue;
              return (
                <div key={key}>
                  <p className="text-[10px] text-muted-foreground">{key}</p>
                  <button
                    type="button"
                    onClick={() => navigator.clipboard.writeText(strValue)}
                    className="group break-all text-left text-[11px] transition-colors hover:text-primary"
                    title="Copy"
                  >
                    {display}
                    <Copy className="ml-1 inline h-2.5 w-2.5 opacity-0 group-hover:opacity-60" />
                  </button>
                </div>
              );
            })}
        </div>
      </div>

      {/* Neighbours */}
      {neighbours.length > 0 && (
        <div>
          <p className="mb-1 flex items-center justify-between text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            <span>Neighbours</span>
            <span className="font-mono">{neighbours.length}</span>
          </p>
          <div className="space-y-px">
            {neighbours.map((n) => (
              <button
                key={n.relId}
                type="button"
                onClick={() => onSelectComposite(n.compositeId)}
                className="flex w-full items-center gap-2 rounded border-l-2 px-2 py-1 text-left text-[11px] transition-colors hover:bg-accent/60"
                style={{ borderColor: n.color }}
                title={n.relType}
              >
                {n.direction === "out" ? (
                  <ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                ) : (
                  <ArrowLeft className="h-3 w-3 shrink-0 text-muted-foreground" />
                )}
                <span className="truncate">{n.name}</span>
                <span className="ml-auto truncate font-mono text-[9.5px] text-muted-foreground">
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

function Checkbox({ checked, color }: { checked: boolean; color: string }) {
  return (
    <span
      aria-hidden
      className="relative inline-block h-3 w-3 shrink-0 rounded-[3px] border transition-colors"
      style={{
        background: checked ? color : "transparent",
        borderColor: checked ? color : "hsl(var(--border))",
      }}
    >
      {checked && (
        <span
          className="absolute"
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
      className="h-2.5 w-2.5 shrink-0 rounded-full transition-opacity"
      style={{ backgroundColor: color, opacity: dim ? 0.3 : 1 }}
    />
  );
}
