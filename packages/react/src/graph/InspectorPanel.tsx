/**
 * InspectorPanel — docked glass sidebar, right edge, mounted only while a
 * node is selected. Docks beside the canvas (the canvas shrinks) rather than
 * floating over it, so the panel never covers the graph. Properties,
 * metadata, and incident edges grouped by relationship type with direction
 * glyphs; per-edge fly-to; focus-neighborhood + copy-id actions.
 */
import { useMemo, useState } from "react";
import { ArrowLeft, ArrowRight, ChevronDown, ChevronRight, Copy, Crosshair, Maximize2, X } from "lucide-react";
import type { GraphNode, GraphRelationship } from "@pensieve-ai/client";
import { useGraphStore } from "./graph-store";
import { getRelationshipFamilyColor } from "./graph-style";
import { NodeDetailModal } from "./NodeDetailModal";
import { nodeName, nodeSubtitle, orderedProps, formatValue } from "./node-detail";

const keyOf = (n: { id: string; namespace?: string }) => `${n.namespace ?? ""}::${n.id}`;

export function InspectorPanel({
  node,
  edges,
  nodesByCompositeId,
}: {
  node: GraphNode | null;
  edges: GraphRelationship[];
  nodesByCompositeId: Map<string, GraphNode>;
}) {
  const selectNode = useGraphStore((s) => s.selectNode);
  const focusNode = useGraphStore((s) => s.focusNode);
  const pushTrail = useGraphStore((s) => s.pushTrail);
  const focusModeId = useGraphStore((s) => s.focusModeId);
  const setFocusMode = useGraphStore((s) => s.setFocusMode);
  const [detailOpen, setDetailOpen] = useState(false);

  const incident = useMemo(() => {
    if (!node) return new Map<string, Array<{ edge: GraphRelationship; otherKey: string; out: boolean }>>();
    const key = keyOf(node);
    const groups = new Map<string, Array<{ edge: GraphRelationship; otherKey: string; out: boolean }>>();
    for (const e of edges) {
      const src = `${e.namespace ?? ""}::${e.source_id}`;
      const dst = `${(e.properties?.target_namespace as string | undefined) ?? e.namespace ?? ""}::${e.target_id}`;
      if (src !== key && dst !== key) continue;
      const out = src === key;
      const arr = groups.get(e.relationship_type) ?? [];
      arr.push({ edge: e, otherKey: out ? dst : src, out });
      groups.set(e.relationship_type, arr);
    }
    return groups;
  }, [node, edges]);

  if (!node) return null;
  const nodeKey = keyOf(node);
  const name = nodeName(node);

  return (
    <div className="pv-h-full pv-w-80 pv-max-w-[85vw] pv-shrink-0 pv-overflow-y-auto pv-border-l pv-border-border pv-glass pv-animate-fade-in">
      <div className="pv-flex pv-items-start pv-justify-between pv-border-b pv-border-border pv-p-3">
        <div className="pv-min-w-0">
          <div className="pv-truncate pv-text-sm pv-font-medium pv-text-foreground">{name}</div>
          <div className="pv-text-2xs pv-text-muted-foreground">{nodeSubtitle(node)}</div>
        </div>
        <button type="button" onClick={() => selectNode(null)} className="pv-text-muted-foreground hover:pv-text-foreground">
          <X className="pv-h-4 pv-w-4" />
        </button>
      </div>

      <div className="pv-flex pv-gap-2 pv-p-3">
        <button
          type="button"
          onClick={() => setDetailOpen(true)}
          className="pv-flex pv-items-center pv-gap-1 pv-rounded-md pv-border pv-border-border pv-px-2 pv-py-1 pv-text-xs pv-text-muted-foreground hover:pv-text-foreground"
        >
          <Maximize2 className="pv-h-3 pv-w-3" /> View details
        </button>
        <button
          type="button"
          onClick={() => setFocusMode(focusModeId === nodeKey ? null : nodeKey)}
          className="pv-flex pv-items-center pv-gap-1 pv-rounded-md pv-border pv-border-border pv-px-2 pv-py-1 pv-text-xs pv-text-muted-foreground hover:pv-text-foreground"
        >
          <Crosshair className="pv-h-3 pv-w-3" />
          {focusModeId === nodeKey ? "Exit focus" : "Focus neighborhood"}
        </button>
        <button
          type="button"
          onClick={() => void navigator.clipboard.writeText(node.id)}
          className="pv-flex pv-items-center pv-gap-1 pv-rounded-md pv-border pv-border-border pv-px-2 pv-py-1 pv-text-xs pv-text-muted-foreground hover:pv-text-foreground"
        >
          <Copy className="pv-h-3 pv-w-3" /> Copy id
        </button>
      </div>

      {Object.keys(node.properties ?? {}).length > 0 && (
        <div className="pv-border-t pv-border-border pv-p-3">
          <div className="pv-mb-1 pv-text-2xs pv-uppercase pv-text-muted-foreground">Properties</div>
          <dl className="pv-space-y-1">
            {orderedProps(node).map(([k, v]) => (
              <PropertyRow key={k} propKey={k} value={v} />
            ))}
          </dl>
        </div>
      )}

      <div className="pv-border-t pv-border-border pv-p-3">
        <div className="pv-mb-1 pv-text-2xs pv-uppercase pv-text-muted-foreground">Relationships</div>
        {[...incident.entries()].map(([relType, items]) => (
          <div key={relType} className="pv-mb-2">
            <div className="pv-flex pv-items-center pv-gap-1.5 pv-text-2xs pv-font-mono" style={{ color: getRelationshipFamilyColor(relType) }}>
              {relType} <span className="pv-text-muted-foreground">({items.length})</span>
            </div>
            <ul className="pv-mt-0.5 pv-space-y-0.5">
              {items.slice(0, 25).map(({ edge, otherKey, out }) => {
                const other = nodesByCompositeId.get(otherKey);
                const otherName = other
                  ? (other.properties?.name as string) || other.id
                  : otherKey.split("::").pop();
                return (
                  <li key={edge.id}>
                    <button
                      type="button"
                      onClick={() => {
                        pushTrail(otherKey);
                        focusNode(otherKey);
                      }}
                      className="pv-flex pv-w-full pv-items-center pv-gap-1.5 pv-rounded pv-px-1 pv-py-0.5 pv-text-left pv-text-xs pv-text-muted-foreground hover:pv-bg-accent hover:pv-text-foreground"
                    >
                      {out ? <ArrowRight className="pv-h-3 pv-w-3 pv-shrink-0" /> : <ArrowLeft className="pv-h-3 pv-w-3 pv-shrink-0" />}
                      <span className="pv-truncate">{otherName}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
        {incident.size === 0 && <div className="pv-text-xs pv-text-muted-foreground">No edges.</div>}
      </div>

      <NodeDetailModal key={node.id} node={node} open={detailOpen} onClose={() => setDetailOpen(false)} />
    </div>
  );
}

/** One property row in the docked panel: truncated by default, expandable to the
 * full wrapped value, with a hover copy button. */
function PropertyRow({ propKey, value }: { propKey: string; value: unknown }) {
  const [open, setOpen] = useState(false);
  const full = formatValue(value).text;
  return (
    <div className="pv-group pv-flex pv-items-start pv-gap-1 pv-text-xs">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="pv-mt-0.5 pv-text-muted-foreground hover:pv-text-foreground"
        aria-label={`${open ? "collapse" : "expand"} ${propKey}`}
      >
        {open ? <ChevronDown className="pv-h-3 pv-w-3" /> : <ChevronRight className="pv-h-3 pv-w-3" />}
      </button>
      <dt className="pv-w-24 pv-shrink-0 pv-truncate pv-text-muted-foreground" title={propKey}>
        {propKey}
      </dt>
      <dd
        className={`pv-min-w-0 pv-flex-1 pv-font-mono pv-text-foreground ${
          open ? "pv-whitespace-pre-wrap pv-break-words" : "pv-truncate"
        }`}
      >
        {full}
      </dd>
      <button
        type="button"
        onClick={() => void navigator.clipboard.writeText(full)}
        className="pv-text-muted-foreground pv-opacity-0 group-hover:pv-opacity-100 hover:pv-text-foreground"
        aria-label={`copy ${propKey}`}
      >
        <Copy className="pv-h-3 pv-w-3" />
      </button>
    </div>
  );
}
