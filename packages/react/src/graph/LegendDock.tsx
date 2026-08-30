/**
 * LegendDock — bottom-center collapsed chip → expandable glass panel.
 * Collapsed: relationship-family color swatches + node/edge counts.
 * Expanded: node-type visibility, namespace visibility, relationship-type
 * isolation, 6 style toggles, and 4-button layout picker.
 */
import { useState, useMemo } from "react";
import {
  Sun,
  Waypoints,
  Spline,
  Circle as CircleIcon,
  LayoutGrid,
  Tag as TagIcon,
  Eye,
  EyeOff,
  Layers,
  Radio,
  ChevronDown,
  ChevronUp,
} from "lucide-react";
import type { GraphStats, LayoutAlgorithm } from "@pensieve-ai/client";
import { getLabelColor } from "@pensieve-ai/client";
import type { GraphCoord } from "../hooks/usePensieveGraph";
import { useGraphStore } from "./graph-store";
import {
  getRelationshipFamilyColor,
  FAMILY_COLORS,
  FAMILY_LABEL,
} from "./graph-style";
import { resolveGraphIcon } from "./graph-icons";
import { cn } from "../internal/cn";

const LAYOUTS: { value: LayoutAlgorithm; label: string }[] = [
  { value: "force", label: "Force" },
  { value: "tree", label: "Tree" },
  { value: "radial", label: "Radial" },
  { value: "grid", label: "Grid" },
];

// The 6 distinct relationship families shown as color swatches in the chip.
const FAMILY_SWATCH_ENTRIES = Object.entries(FAMILY_COLORS) as [keyof typeof FAMILY_COLORS, string][];

export function LegendDock(props: {
  stats: GraphStats | null;
  coords: GraphCoord[];
  namespaceCounts: Record<string, number>;
  visibleNodes: number;
  visibleEdges: number;
}): JSX.Element {
  const [expanded, setExpanded] = useState(false);

  const layout = useGraphStore((s) => s.layout);
  const setLayout = useGraphStore((s) => s.setLayout);
  const hiddenLabels = useGraphStore((s) => s.hiddenLabels);
  const toggleHiddenLabel = useGraphStore((s) => s.toggleHiddenLabel);
  const hiddenNamespaces = useGraphStore((s) => s.hiddenNamespaces);
  const toggleHiddenNamespace = useGraphStore((s) => s.toggleHiddenNamespace);
  const setHiddenNamespaces = useGraphStore((s) => s.setHiddenNamespaces);
  const relTypeFilter = useGraphStore((s) => s.relTypeFilter);
  const setRelTypeFilter = useGraphStore((s) => s.setRelTypeFilter);
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
  const showEdgeLabels = useGraphStore((s) => s.showEdgeLabels);
  const toggleEdgeLabels = useGraphStore((s) => s.toggleEdgeLabels);
  const showLiveConsumers = useGraphStore((s) => s.showLiveConsumers);
  const toggleLiveConsumers = useGraphStore((s) => s.toggleLiveConsumers);

  const labelEntries = useMemo(() => {
    if (!props.stats) return [] as [string, number][];
    return Object.entries(props.stats.label_counts).sort(([, a], [, b]) => b - a);
  }, [props.stats]);

  const relEntries = useMemo(() => {
    if (!props.stats) return [] as [string, number][];
    return Object.entries(props.stats.relationship_type_counts).sort(([, a], [, b]) => b - a);
  }, [props.stats]);

  const namespaceEntries = useMemo(() => {
    return Object.entries(props.namespaceCounts).sort(([, a], [, b]) => b - a);
  }, [props.namespaceCounts]);

  const noNamespaceHidden = hiddenNamespaces.length === 0;
  const showNamespaceFilter = namespaceEntries.length > 1;

  return (
    <div className="pv-absolute pv-bottom-4 pv-left-1/2 pv-z-20 -pv-translate-x-1/2 pv-flex pv-flex-col pv-items-center pv-gap-1">
      {/* Expanded panel */}
      {expanded && (
        <div className="pv-glass pv-rounded-xl pv-border pv-border-border pv-shadow-elev-3 pv-w-72 pv-max-h-96 pv-overflow-y-auto pv-p-3 pv-space-y-4">
          {/* Node-type visibility */}
          {labelEntries.length > 0 && (
            <div>
              <span className="pv-mb-1.5 pv-block pv-text-[10px] pv-font-semibold pv-uppercase pv-tracking-wider pv-text-muted-foreground">
                Node types
              </span>
              <div className="pv-space-y-px">
                {labelEntries.map(([label, count]) => {
                  const isHidden = hiddenLabels.includes(label);
                  const color = getLabelColor(label);
                  const ic = resolveGraphIcon([label], {});
                  return (
                    <button
                      key={label}
                      type="button"
                      onClick={() => toggleHiddenLabel(label)}
                      className={cn(
                        "pv-group pv-flex pv-w-full pv-items-center pv-gap-2 pv-rounded pv-px-1.5 pv-py-1 pv-text-left pv-text-xs pv-transition-colors",
                        isHidden
                          ? "pv-text-muted-foreground/60"
                          : "hover:pv-bg-accent/60 hover:pv-text-accent-foreground",
                      )}
                    >
                      {ic ? (
                        <ic.Comp size={12} color={color} className={cn("pv-shrink-0", isHidden && "pv-opacity-40")} />
                      ) : (
                        <span
                          className="pv-h-2 pv-w-2 pv-shrink-0 pv-rounded-full"
                          style={{ backgroundColor: isHidden ? "#94a3b8" : color }}
                        />
                      )}
                      <span className={cn("pv-flex-1 pv-truncate", isHidden && "pv-line-through")}>
                        {label}
                      </span>
                      <span className="pv-font-mono pv-text-[10px] pv-tabular-nums pv-text-muted-foreground">
                        {count.toLocaleString()}
                      </span>
                      {isHidden
                        ? <EyeOff className="pv-h-3 pv-w-3 pv-shrink-0 pv-text-muted-foreground/70" />
                        : <Eye className="pv-h-3 pv-w-3 pv-shrink-0 pv-opacity-0 pv-transition-opacity group-hover:pv-opacity-50" />}
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          {/* Namespace visibility */}
          {showNamespaceFilter && (
            <div>
              <div className="pv-mb-1.5 pv-flex pv-items-center pv-justify-between">
                <span className="pv-flex pv-items-center pv-gap-1.5 pv-text-[10px] pv-font-semibold pv-uppercase pv-tracking-wider pv-text-muted-foreground">
                  <Layers className="pv-h-3 pv-w-3" /> Graphs
                </span>
                <button
                  type="button"
                  onClick={noNamespaceHidden
                    ? () => setHiddenNamespaces(namespaceEntries.map(([ns]) => ns))
                    : () => setHiddenNamespaces([])}
                  className="pv-text-[10px] pv-font-medium pv-text-primary pv-transition-opacity hover:pv-opacity-80"
                >
                  {noNamespaceHidden ? "Hide all" : "Show all"}
                </button>
              </div>
              <div className="pv-space-y-px">
                {namespaceEntries.map(([ns, count]) => {
                  const isHidden = hiddenNamespaces.includes(ns);
                  const color = getLabelColor(ns);
                  return (
                    <button
                      key={ns}
                      type="button"
                      onClick={() => toggleHiddenNamespace(ns)}
                      className={cn(
                        "pv-group pv-flex pv-w-full pv-items-center pv-gap-2 pv-rounded pv-px-1.5 pv-py-1 pv-text-left pv-text-xs pv-transition-colors",
                        isHidden ? "pv-text-muted-foreground/60" : "hover:pv-bg-accent/60",
                      )}
                    >
                      <span
                        className="pv-h-2 pv-w-2 pv-shrink-0 pv-rounded-full"
                        style={{ backgroundColor: isHidden ? "#94a3b8" : color }}
                      />
                      <span className={cn("pv-flex-1 pv-truncate", isHidden && "pv-line-through")}>{ns}</span>
                      <span className="pv-font-mono pv-text-[10px] pv-tabular-nums pv-text-muted-foreground">
                        {count.toLocaleString()}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          {/* Relationship-type isolation */}
          {relEntries.length > 0 && (
            <div>
              <span className="pv-mb-1.5 pv-block pv-text-[10px] pv-font-semibold pv-uppercase pv-tracking-wider pv-text-muted-foreground">
                Relationships
              </span>
              <div className="pv-space-y-px">
                {relEntries.map(([rel, count]) => {
                  const color = getRelationshipFamilyColor(rel);
                  const isActive = relTypeFilter === rel;
                  return (
                    <button
                      key={rel}
                      type="button"
                      onClick={() => setRelTypeFilter(isActive ? null : rel)}
                      className={cn(
                        "pv-flex pv-w-full pv-items-center pv-gap-2 pv-rounded pv-px-1.5 pv-py-1 pv-text-left pv-text-xs pv-transition-colors",
                        isActive
                          ? "pv-bg-accent pv-text-accent-foreground"
                          : "pv-text-muted-foreground hover:pv-bg-accent/60 hover:pv-text-foreground",
                      )}
                    >
                      <span className="pv-h-[3px] pv-w-3.5 pv-shrink-0 pv-rounded-full" style={{ backgroundColor: color }} />
                      <span className="pv-flex-1 pv-truncate">{rel}</span>
                      <span className="pv-font-mono pv-text-[10px] pv-tabular-nums pv-text-muted-foreground">
                        {count.toLocaleString()}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          {/* Style toggles */}
          <div>
            <span className="pv-mb-1.5 pv-block pv-text-[10px] pv-font-semibold pv-uppercase pv-tracking-wider pv-text-muted-foreground">
              Style
            </span>
            <div className="pv-grid pv-grid-cols-3 pv-gap-1">
              <StyleToggle icon={Sun} label="Glow" on={glow} onClick={toggleGlow} title="Glow halos on focus/landmark nodes" />
              <StyleToggle icon={Waypoints} label="Flow" on={animatedFlow} onClick={toggleAnimatedFlow} title="Animated flow on focused edges" />
              <StyleToggle icon={Spline} label="Curve" on={curvedEdges} onClick={toggleCurvedEdges} title="Curve edges" />
              <StyleToggle icon={LayoutGrid} label="Clusters" on={communityHulls} onClick={toggleCommunityHulls} title="Translucent community hulls" />
              <StyleToggle icon={CircleIcon} label="Size" on={sizeByDegree} onClick={toggleSizeByDegree} title="Size nodes by degree" />
              <StyleToggle icon={TagIcon} label="Labels" on={showEdgeLabels} onClick={toggleEdgeLabels} title="Relationship labels on edges" />
              <StyleToggle icon={Radio} label="Live" on={showLiveConsumers} onClick={toggleLiveConsumers} title="Show live consumers — agents reading/writing memory in realtime" />
            </div>
          </div>

          {/* Layout picker */}
          <div>
            <span className="pv-mb-1.5 pv-block pv-text-[10px] pv-font-semibold pv-uppercase pv-tracking-wider pv-text-muted-foreground">
              Layout
            </span>
            <div className="pv-flex pv-items-center pv-gap-1">
              {LAYOUTS.map((l) => (
                <button
                  key={l.value}
                  type="button"
                  onClick={() => setLayout(l.value)}
                  className={cn(
                    "pv-flex-1 pv-rounded pv-px-2 pv-py-1 pv-text-[10px] pv-font-medium pv-transition-colors",
                    layout === l.value
                      ? "pv-bg-primary pv-text-primary-foreground"
                      : "pv-border pv-border-transparent pv-text-muted-foreground hover:pv-border-border hover:pv-text-foreground",
                  )}
                >
                  {l.label}
                </button>
              ))}
            </div>
          </div>

          {/* Family legend */}
          <div>
            <span className="pv-mb-1.5 pv-block pv-text-[10px] pv-font-semibold pv-uppercase pv-tracking-wider pv-text-muted-foreground">
              Edge families
            </span>
            <div className="pv-flex pv-flex-wrap pv-gap-2">
              {FAMILY_SWATCH_ENTRIES.map(([family, color]) => (
                <div key={family} className="pv-flex pv-items-center pv-gap-1">
                  <span className="pv-h-[3px] pv-w-4 pv-rounded-full" style={{ backgroundColor: color }} />
                  <span className="pv-text-[10px] pv-text-muted-foreground">{FAMILY_LABEL[family]}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      {/* Collapsed chip */}
      <button
        type="button"
        onClick={() => setExpanded((e) => !e)}
        className="pv-glass pv-flex pv-items-center pv-gap-2 pv-rounded-full pv-border pv-border-border pv-px-3 pv-py-1.5 pv-shadow-elev-2 hover:pv-border-primary/50 pv-transition-colors"
      >
        {/* Family color swatches */}
        <span className="pv-flex pv-gap-0.5 pv-items-center">
          {FAMILY_SWATCH_ENTRIES.map(([family, color]) => (
            <span
              key={family}
              className="pv-h-2 pv-w-2 pv-rounded-full"
              style={{ backgroundColor: color }}
              title={FAMILY_LABEL[family]}
            />
          ))}
        </span>
        <span className="pv-text-xs pv-text-muted-foreground">
          {props.visibleNodes.toLocaleString()} nodes · {props.visibleEdges.toLocaleString()} edges
        </span>
        {expanded
          ? <ChevronDown className="pv-h-3 pv-w-3 pv-text-muted-foreground" />
          : <ChevronUp className="pv-h-3 pv-w-3 pv-text-muted-foreground" />}
      </button>
    </div>
  );
}

/* ── internal helpers ──────────────────────────────────────────────────────── */

function StyleToggle({
  icon: Icon,
  label,
  on,
  onClick,
  title,
}: {
  icon: React.ComponentType<{ className?: string }>;
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
      className={cn(
        "pv-flex pv-items-center pv-justify-center pv-gap-1 pv-rounded pv-border pv-px-1.5 pv-py-1 pv-text-[10px] pv-transition-colors",
        on
          ? "pv-border-primary/60 pv-bg-primary/10 pv-text-primary"
          : "pv-border-transparent pv-text-muted-foreground hover:pv-border-border hover:pv-text-foreground",
      )}
    >
      <Icon className="pv-h-3 pv-w-3" />
      {label}
    </button>
  );
}
