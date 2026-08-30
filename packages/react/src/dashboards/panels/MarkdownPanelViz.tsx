/**
 * MarkdownPanelViz — renders a markdown-type dashboard panel using the shared
 * minimal markdown renderer (packages/react/src/internal/ui/markdown.tsx).
 */

import type { DashboardPanel } from "@pensieve-ai/client";
import { renderMarkdown } from "../../internal/ui/markdown";

interface Props {
  panel: DashboardPanel;
}

export function MarkdownPanelViz({ panel }: Props) {
  const markdown = (panel.config.markdown as string | undefined) ?? "";

  if (!markdown.trim()) {
    return (
      <div className="pv-flex pv-h-full pv-items-center pv-justify-center pv-text-xs pv-text-muted-foreground">
        No content.
      </div>
    );
  }

  return <div className="pv-h-full pv-overflow-y-auto pv-px-3 pv-py-2">{renderMarkdown(markdown)}</div>;
}
