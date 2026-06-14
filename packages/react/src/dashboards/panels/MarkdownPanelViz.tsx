/**
 * MarkdownPanelViz — renders a markdown-type dashboard panel using the shared
 * minimal markdown renderer (packages/react/src/internal/ui/markdown.tsx).
 */

import type { DashboardPanel } from "@kyma-ai/client";
import { renderMarkdown } from "../../internal/ui/markdown";

interface Props {
  panel: DashboardPanel;
}

export function MarkdownPanelViz({ panel }: Props) {
  const markdown = (panel.config.markdown as string | undefined) ?? "";

  if (!markdown.trim()) {
    return (
      <div className="ky-flex ky-h-full ky-items-center ky-justify-center ky-text-xs ky-text-muted-foreground">
        No content.
      </div>
    );
  }

  return <div className="ky-h-full ky-overflow-y-auto ky-px-3 ky-py-2">{renderMarkdown(markdown)}</div>;
}
