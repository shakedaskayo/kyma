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
    <div className="ky-h-full ky-w-80 ky-max-w-[85vw] ky-shrink-0 ky-overflow-y-auto ky-border-l ky-border-border ky-glass ky-animate-fade-in">
      <div className="ky-flex ky-items-start ky-justify-between ky-border-b ky-border-border ky-p-3">
        <div className="ky-min-w-0">
          <div className="ky-truncate ky-text-sm ky-font-medium ky-text-foreground">{name}</div>
          <div className="ky-text-2xs ky-text-muted-foreground">{nodeSubtitle(node)}</div>
        </div>
        <button type="button" onClick={() => selectNode(null)} className="ky-text-muted-foreground hover:ky-text-foreground">
          <X className="ky-h-4 ky-w-4" />
        </button>
      </div>

      <div className="ky-flex ky-gap-2 ky-p-3">
        <button
          type="button"
          onClick={() => setDetailOpen(true)}
          className="ky-flex ky-items-center ky-gap-1 ky-rounded-md ky-border ky-border-border ky-px-2 ky-py-1 ky-text-xs ky-text-muted-foreground hover:ky-text-foreground"
        >
          <Maximize2 className="ky-h-3 ky-w-3" /> View details
        </button>
        <button
          type="button"
          onClick={() => setFocusMode(focusModeId === nodeKey ? null : nodeKey)}
          className="ky-flex ky-items-center ky-gap-1 ky-rounded-md ky-border ky-border-border ky-px-2 ky-py-1 ky-text-xs ky-text-muted-foreground hover:ky-text-foreground"
        >
          <Crosshair className="ky-h-3 ky-w-3" />
          {focusModeId === nodeKey ? "Exit focus" : "Focus neighborhood"}
        </button>
        <button
          type="button"
          onClick={() => void navigator.clipboard.writeText(node.id)}
          className="ky-flex ky-items-center ky-gap-1 ky-rounded-md ky-border ky-border-border ky-px-2 ky-py-1 ky-text-xs ky-text-muted-foreground hover:ky-text-foreground"
        >
          <Copy className="ky-h-3 ky-w-3" /> Copy id
        </button>
      </div>

      {Object.keys(node.properties ?? {}).length > 0 && (
        <div className="ky-border-t ky-border-border ky-p-3">
          <div className="ky-mb-1 ky-text-2xs ky-uppercase ky-text-muted-foreground">Properties</div>
          <dl className="ky-space-y-1">
            {orderedProps(node).map(([k, v]) => (
              <PropertyRow key={k} propKey={k} value={v} />
            ))}
          </dl>
        </div>
      )}

      <div className="ky-border-t ky-border-border ky-p-3">
        <div className="ky-mb-1 ky-text-2xs ky-uppercase ky-text-muted-foreground">Relationships</div>
        {[...incident.entries()].map(([relType, items]) => (
          <div key={relType} className="ky-mb-2">
            <div className="ky-flex ky-items-center ky-gap-1.5 ky-text-2xs ky-font-mono" style={{ color: getRelationshipFamilyColor(relType) }}>
              {relType} <span className="ky-text-muted-foreground">({items.length})</span>
            </div>
            <ul className="ky-mt-0.5 ky-space-y-0.5">
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
                      className="ky-flex ky-w-full ky-items-center ky-gap-1.5 ky-rounded ky-px-1 ky-py-0.5 ky-text-left ky-text-xs ky-text-muted-foreground hover:ky-bg-accent hover:ky-text-foreground"
                    >
                      {out ? <ArrowRight className="ky-h-3 ky-w-3 ky-shrink-0" /> : <ArrowLeft className="ky-h-3 ky-w-3 ky-shrink-0" />}
                      <span className="ky-truncate">{otherName}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
        {incident.size === 0 && <div className="ky-text-xs ky-text-muted-foreground">No edges.</div>}
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
    <div className="ky-group ky-flex ky-items-start ky-gap-1 ky-text-xs">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="ky-mt-0.5 ky-text-muted-foreground hover:ky-text-foreground"
        aria-label={`${open ? "collapse" : "expand"} ${propKey}`}
      >
        {open ? <ChevronDown className="ky-h-3 ky-w-3" /> : <ChevronRight className="ky-h-3 ky-w-3" />}
      </button>
      <dt className="ky-w-24 ky-shrink-0 ky-truncate ky-text-muted-foreground" title={propKey}>
        {propKey}
      </dt>
      <dd
        className={`ky-min-w-0 ky-flex-1 ky-font-mono ky-text-foreground ${
          open ? "ky-whitespace-pre-wrap ky-break-words" : "ky-truncate"
        }`}
      >
        {full}
      </dd>
      <button
        type="button"
        onClick={() => void navigator.clipboard.writeText(full)}
        className="ky-text-muted-foreground ky-opacity-0 group-hover:ky-opacity-100 hover:ky-text-foreground"
        aria-label={`copy ${propKey}`}
      >
        <Copy className="ky-h-3 ky-w-3" />
      </button>
    </div>
  );
}
