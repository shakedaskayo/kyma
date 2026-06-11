/**
 * InspectorPanel — floating glass panel, right side, mounted only while a
 * node is selected. Properties, metadata, and incident edges grouped by
 * relationship type with direction glyphs; per-edge fly-to; focus-
 * neighborhood + copy-id actions.
 */
import { useMemo } from "react";
import { ArrowLeft, ArrowRight, Copy, Crosshair, X } from "lucide-react";
import type { GraphNode, GraphRelationship } from "@kyma-ai/client";
import { useGraphStore } from "./graph-store";
import { getRelationshipFamilyColor } from "./graph-style";

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
  const name = (node.properties?.name as string) || (node.properties?.title as string) || node.id;

  return (
    <div className="ky-absolute ky-right-4 ky-top-16 ky-bottom-20 ky-z-20 ky-w-80 ky-overflow-y-auto ky-rounded-xl ky-border ky-border-border ky-glass ky-shadow-elev-3 ky-animate-fade-in">
      <div className="ky-flex ky-items-start ky-justify-between ky-border-b ky-border-border ky-p-3">
        <div className="ky-min-w-0">
          <div className="ky-truncate ky-text-sm ky-font-medium ky-text-foreground">{name}</div>
          <div className="ky-text-2xs ky-text-muted-foreground">
            {node.labels.join(" · ")} · {node.namespace}
          </div>
        </div>
        <button type="button" onClick={() => selectNode(null)} className="ky-text-muted-foreground hover:ky-text-foreground">
          <X className="ky-h-4 ky-w-4" />
        </button>
      </div>

      <div className="ky-flex ky-gap-2 ky-p-3">
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
            {Object.entries(node.properties).slice(0, 30).map(([k, v]) => (
              <div key={k} className="ky-flex ky-gap-2 ky-text-xs">
                <dt className="ky-w-28 ky-shrink-0 ky-truncate ky-text-muted-foreground">{k}</dt>
                <dd className="ky-min-w-0 ky-truncate ky-font-mono ky-text-foreground">{String(v)}</dd>
              </div>
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
    </div>
  );
}
