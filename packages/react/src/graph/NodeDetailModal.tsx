/**
 * NodeDetailModal — wide modal for a graph node. Renders the node's `content`
 * as markdown (raw/rendered toggle), every property untruncated with per-value
 * copy, and (when the node references a stored artifact via `object_path`) a
 * paged source-file viewer. Built on the internal Radix Dialog primitive.
 */
import { useState } from "react";
import { Copy } from "lucide-react";
import type { GraphNode } from "@kyma-ai/client";
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
      <DialogContent className="ky-max-w-3xl ky-max-h-[85vh] ky-overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="ky-truncate ky-pr-8">{name}</DialogTitle>
          <div className="ky-text-2xs ky-text-muted-foreground">{nodeSubtitle(node)}</div>
        </DialogHeader>

        {content != null && (
          <section className="ky-space-y-1">
            <div className="ky-flex ky-items-center ky-justify-between">
              <div className="ky-text-2xs ky-uppercase ky-text-muted-foreground">Content</div>
              <Button variant="ghost" size="xs" onClick={() => setRaw((r) => !r)}>
                {raw ? "Rendered" : "Raw"}
              </Button>
            </div>
            {raw ? (
              <pre className="ky-overflow-x-auto ky-rounded ky-bg-muted ky-p-3 ky-font-mono ky-text-xs ky-whitespace-pre-wrap ky-break-words">
                {content}
              </pre>
            ) : (
              <div className="ky-rounded ky-border ky-border-border ky-p-3">
                <Markdown source={content} />
              </div>
            )}
          </section>
        )}

        <section className="ky-space-y-1">
          <div className="ky-text-2xs ky-uppercase ky-text-muted-foreground">Properties</div>
          <dl className="ky-space-y-1.5">
            {orderedProps(node).map(([k, v]) => {
              const f = formatValue(v);
              return (
                <div key={k} className="ky-group ky-grid ky-grid-cols-[140px_1fr] ky-gap-2 ky-text-xs">
                  <dt className="ky-truncate ky-text-muted-foreground" title={k}>
                    {k}
                  </dt>
                  <dd className="ky-flex ky-min-w-0 ky-items-start ky-gap-1">
                    <span className="ky-min-w-0 ky-flex-1 ky-whitespace-pre-wrap ky-break-words ky-font-mono ky-text-foreground">
                      {f.text}
                    </span>
                    <button
                      type="button"
                      onClick={() => void navigator.clipboard.writeText(f.text)}
                      className="ky-text-muted-foreground ky-opacity-0 group-hover:ky-opacity-100 hover:ky-text-foreground"
                      aria-label={`copy ${k}`}
                    >
                      <Copy className="ky-h-3 ky-w-3" />
                    </button>
                  </dd>
                </div>
              );
            })}
          </dl>
        </section>

        {sourcePath != null && (
          <section className="ky-space-y-1">
            <div className="ky-text-2xs ky-uppercase ky-text-muted-foreground">Source file</div>
            <ArtifactSourceViewer path={sourcePath} />
          </section>
        )}
      </DialogContent>
    </Dialog>
  );
}
