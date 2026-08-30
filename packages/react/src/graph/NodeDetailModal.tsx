/**
 * NodeDetailModal — wide modal for a graph node. Renders the node's `content`
 * as markdown (raw/rendered toggle), every property untruncated with per-value
 * copy, and (when the node references a stored artifact via `object_path`) a
 * paged source-file viewer. Built on the internal Radix Dialog primitive.
 */
import { useState } from "react";
import { Copy } from "lucide-react";
import type { GraphNode } from "@pensieve-ai/client";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../internal/ui/dialog";
import { Button } from "../internal/ui/button";
import { Markdown } from "../internal/ui/markdown";
import { ArtifactSourceViewer } from "./ArtifactSourceViewer";
import { nodeContent, nodeName, nodeSourcePath, nodeSubtitle, orderedProps, formatValue } from "./node-detail";

export function NodeDetailModal({
  node,
  open,
  onClose,
}: {
  node: GraphNode | null;
  open: boolean;
  onClose: () => void;
}) {
  const [raw, setRaw] = useState(false);
  if (!node) return null;

  const name = nodeName(node);
  const content = nodeContent(node);
  const sourcePath = nodeSourcePath(node);

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
    >
      <DialogContent className="pv-max-w-3xl pv-max-h-[85vh] pv-overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="pv-truncate pv-pr-8">{name}</DialogTitle>
          <div className="pv-text-2xs pv-text-muted-foreground">{nodeSubtitle(node)}</div>
        </DialogHeader>

        {content != null && (
          <section className="pv-space-y-1">
            <div className="pv-flex pv-items-center pv-justify-between">
              <div className="pv-text-2xs pv-uppercase pv-text-muted-foreground">Content</div>
              <Button variant="ghost" size="xs" onClick={() => setRaw((r) => !r)}>
                {raw ? "Rendered" : "Raw"}
              </Button>
            </div>
            {raw ? (
              <pre className="pv-overflow-x-auto pv-rounded pv-bg-muted pv-p-3 pv-font-mono pv-text-xs pv-whitespace-pre-wrap pv-break-words">
                {content}
              </pre>
            ) : (
              <div className="pv-rounded pv-border pv-border-border pv-p-3">
                <Markdown source={content} />
              </div>
            )}
          </section>
        )}

        <section className="pv-space-y-1">
          <div className="pv-text-2xs pv-uppercase pv-text-muted-foreground">Properties</div>
          <dl className="pv-space-y-1.5">
            {orderedProps(node).map(([k, v]) => {
              const f = formatValue(v);
              return (
                <div key={k} className="pv-group pv-grid pv-grid-cols-[140px_1fr] pv-gap-2 pv-text-xs">
                  <dt className="pv-truncate pv-text-muted-foreground" title={k}>
                    {k}
                  </dt>
                  <dd className="pv-flex pv-min-w-0 pv-items-start pv-gap-1">
                    <span className="pv-min-w-0 pv-flex-1 pv-whitespace-pre-wrap pv-break-words pv-font-mono pv-text-foreground">
                      {f.text}
                    </span>
                    <button
                      type="button"
                      onClick={() => void navigator.clipboard.writeText(f.text)}
                      className="pv-text-muted-foreground pv-opacity-0 group-hover:pv-opacity-100 hover:pv-text-foreground"
                      aria-label={`copy ${k}`}
                    >
                      <Copy className="pv-h-3 pv-w-3" />
                    </button>
                  </dd>
                </div>
              );
            })}
          </dl>
        </section>

        {sourcePath != null && (
          <section className="pv-space-y-1">
            <div className="pv-text-2xs pv-uppercase pv-text-muted-foreground">Source file</div>
            <ArtifactSourceViewer path={sourcePath} />
          </section>
        )}
      </DialogContent>
    </Dialog>
  );
}
