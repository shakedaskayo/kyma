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
  ChevronDown,
  ChevronUp,
} from "lucide-react";
import type { GraphStats, LayoutAlgorithm } from "@kyma-ai/client";
import { getLabelColor } from "@kyma-ai/client";
import type { GraphCoord } from "../hooks/useKymaGraph";
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
    <div className="ky-absolute ky-bottom-4 ky-left-1/2 ky-z-20 -ky-translate-x-1/2 ky-flex ky-flex-col ky-items-center ky-gap-1">
      {/* Expanded panel */}
      {expanded && (
        <div className="ky-glass ky-rounded-xl ky-border ky-border-border ky-shadow-elev-3 ky-w-72 ky-max-h-96 ky-overflow-y-auto ky-p-3 ky-space-y-4">
          {/* Node-type visibility */}
          {labelEntries.length > 0 && (
            <div>
              <span className="ky-mb-1.5 ky-block ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
                Node types
              </span>
              <div className="ky-space-y-px">
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
                        "ky-group ky-flex ky-w-full ky-items-center ky-gap-2 ky-rounded ky-px-1.5 ky-py-1 ky-text-left ky-text-xs ky-transition-colors",
                        isHidden
                          ? "ky-text-muted-foreground/60"
                          : "hover:ky-bg-accent/60 hover:ky-text-accent-foreground",
                      )}
                    >
                      {ic ? (
                        <ic.Comp size={12} color={color} className={cn("ky-shrink-0", isHidden && "ky-opacity-40")} />
                      ) : (
                        <span
                          className="ky-h-2 ky-w-2 ky-shrink-0 ky-rounded-full"
                          style={{ backgroundColor: isHidden ? "#94a3b8" : color }}
                        />
                      )}
                      <span className={cn("ky-flex-1 ky-truncate", isHidden && "ky-line-through")}>
                        {label}
                      </span>
                      <span className="ky-font-mono ky-text-[10px] ky-tabular-nums ky-text-muted-foreground">
                        {count.toLocaleString()}
                      </span>
                      {isHidden
                        ? <EyeOff className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground/70" />
                        : <Eye className="ky-h-3 ky-w-3 ky-shrink-0 ky-opacity-0 ky-transition-opacity group-hover:ky-opacity-50" />}
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          {/* Namespace visibility */}
          {showNamespaceFilter && (
            <div>
              <div className="ky-mb-1.5 ky-flex ky-items-center ky-justify-between">
                <span className="ky-flex ky-items-center ky-gap-1.5 ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
                  <Layers className="ky-h-3 ky-w-3" /> Graphs
                </span>
                <button
                  type="button"
                  onClick={noNamespaceHidden
                    ? () => setHiddenNamespaces(namespaceEntries.map(([ns]) => ns))
                    : () => setHiddenNamespaces([])}
                  className="ky-text-[10px] ky-font-medium ky-text-primary ky-transition-opacity hover:ky-opacity-80"
                >
                  {noNamespaceHidden ? "Hide all" : "Show all"}
                </button>
              </div>
              <div className="ky-space-y-px">
                {namespaceEntries.map(([ns, count]) => {
                  const isHidden = hiddenNamespaces.includes(ns);
                  const color = getLabelColor(ns);
                  return (
                    <button
                      key={ns}
                      type="button"
                      onClick={() => toggleHiddenNamespace(ns)}
                      className={cn(
                        "ky-group ky-flex ky-w-full ky-items-center ky-gap-2 ky-rounded ky-px-1.5 ky-py-1 ky-text-left ky-text-xs ky-transition-colors",
                        isHidden ? "ky-text-muted-foreground/60" : "hover:ky-bg-accent/60",
                      )}
                    >
                      <span
                        className="ky-h-2 ky-w-2 ky-shrink-0 ky-rounded-full"
                        style={{ backgroundColor: isHidden ? "#94a3b8" : color }}
                      />
                      <span className={cn("ky-flex-1 ky-truncate", isHidden && "ky-line-through")}>{ns}</span>
                      <span className="ky-font-mono ky-text-[10px] ky-tabular-nums ky-text-muted-foreground">
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
              <span className="ky-mb-1.5 ky-block ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
                Relationships
              </span>
              <div className="ky-space-y-px">
                {relEntries.map(([rel, count]) => {
                  const color = getRelationshipFamilyColor(rel);
                  const isActive = relTypeFilter === rel;
                  return (
                    <button
                      key={rel}
                      type="button"
                      onClick={() => setRelTypeFilter(isActive ? null : rel)}
                      className={cn(
                        "ky-flex ky-w-full ky-items-center ky-gap-2 ky-rounded ky-px-1.5 ky-py-1 ky-text-left ky-text-xs ky-transition-colors",
                        isActive
                          ? "ky-bg-accent ky-text-accent-foreground"
                          : "ky-text-muted-foreground hover:ky-bg-accent/60 hover:ky-text-foreground",
                      )}
                    >
                      <span className="ky-h-[3px] ky-w-3.5 ky-shrink-0 ky-rounded-full" style={{ backgroundColor: color }} />
                      <span className="ky-flex-1 ky-truncate">{rel}</span>
                      <span className="ky-font-mono ky-text-[10px] ky-tabular-nums ky-text-muted-foreground">
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
            <span className="ky-mb-1.5 ky-block ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
              Style
            </span>
            <div className="ky-grid ky-grid-cols-3 ky-gap-1">
              <StyleToggle icon={Sun} label="Glow" on={glow} onClick={toggleGlow} title="Glow halos on focus/landmark nodes" />
              <StyleToggle icon={Waypoints} label="Flow" on={animatedFlow} onClick={toggleAnimatedFlow} title="Animated flow on focused edges" />
              <StyleToggle icon={Spline} label="Curve" on={curvedEdges} onClick={toggleCurvedEdges} title="Curve edges" />
              <StyleToggle icon={LayoutGrid} label="Clusters" on={communityHulls} onClick={toggleCommunityHulls} title="Translucent community hulls" />
              <StyleToggle icon={CircleIcon} label="Size" on={sizeByDegree} onClick={toggleSizeByDegree} title="Size nodes by degree" />
              <StyleToggle icon={TagIcon} label="Labels" on={showEdgeLabels} onClick={toggleEdgeLabels} title="Relationship labels on edges" />
            </div>
          </div>

          {/* Layout picker */}
          <div>
            <span className="ky-mb-1.5 ky-block ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
              Layout
            </span>
            <div className="ky-flex ky-items-center ky-gap-1">
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
          </div>

          {/* Family legend */}
          <div>
            <span className="ky-mb-1.5 ky-block ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
              Edge families
            </span>
            <div className="ky-flex ky-flex-wrap ky-gap-2">
              {FAMILY_SWATCH_ENTRIES.map(([family, color]) => (
                <div key={family} className="ky-flex ky-items-center ky-gap-1">
                  <span className="ky-h-[3px] ky-w-4 ky-rounded-full" style={{ backgroundColor: color }} />
                  <span className="ky-text-[10px] ky-text-muted-foreground">{FAMILY_LABEL[family]}</span>
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
        className="ky-glass ky-flex ky-items-center ky-gap-2 ky-rounded-full ky-border ky-border-border ky-px-3 ky-py-1.5 ky-shadow-elev-2 hover:ky-border-primary/50 ky-transition-colors"
      >
        {/* Family color swatches */}
        <span className="ky-flex ky-gap-0.5 ky-items-center">
          {FAMILY_SWATCH_ENTRIES.map(([family, color]) => (
            <span
              key={family}
              className="ky-h-2 ky-w-2 ky-rounded-full"
              style={{ backgroundColor: color }}
              title={FAMILY_LABEL[family]}
            />
          ))}
        </span>
        <span className="ky-text-xs ky-text-muted-foreground">
          {props.visibleNodes.toLocaleString()} nodes · {props.visibleEdges.toLocaleString()} edges
        </span>
        {expanded
          ? <ChevronDown className="ky-h-3 ky-w-3 ky-text-muted-foreground" />
          : <ChevronUp className="ky-h-3 ky-w-3 ky-text-muted-foreground" />}
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
        "ky-flex ky-items-center ky-justify-center ky-gap-1 ky-rounded ky-border ky-px-1.5 ky-py-1 ky-text-[10px] ky-transition-colors",
        on
          ? "ky-border-primary/60 ky-bg-primary/10 ky-text-primary"
          : "ky-border-transparent ky-text-muted-foreground hover:ky-border-border hover:ky-text-foreground",
      )}
    >
      <Icon className="ky-h-3 ky-w-3" />
      {label}
    </button>
  );
}
