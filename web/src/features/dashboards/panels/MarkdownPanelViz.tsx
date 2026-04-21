import type { DashboardPanel } from "@/sdk/dashboards";

interface Props {
  panel: DashboardPanel;
}

/**
 * Minimal Markdown renderer.
 * Handles: headings (#/##/###), paragraphs, **bold**, *italic*, `code`,
 * code fences, unordered lists, links.
 */
function renderMarkdown(md: string): React.ReactNode[] {
  const lines = md.split("\n");
  const nodes: React.ReactNode[] = [];
  let i = 0;

  const renderInline = (text: string, key: string): React.ReactNode => {
    // Process inline: **bold**, *italic*, `code`, [link](url)
    const parts: React.ReactNode[] = [];
    let remaining = text;
    let pIdx = 0;

    while (remaining.length > 0) {
      // Bold: **text**
      const boldMatch = remaining.match(/^(.*?)\*\*(.+?)\*\*(.*)/s);
      // Italic: *text* (not **)
      const italicMatch = remaining.match(/^(.*?)(?<!\*)\*(?!\*)(.+?)(?<!\*)\*(?!\*)(.*)/s);
      // Code: `code`
      const codeMatch = remaining.match(/^(.*?)`([^`]+)`(.*)/s);
      // Link: [text](url)
      const linkMatch = remaining.match(/^(.*?)\[([^\]]+)\]\(([^)]+)\)(.*)/s);

      // Find which one comes first
      const candidates: Array<{ idx: number; type: string; match: RegExpMatchArray }> = [];
      if (boldMatch) candidates.push({ idx: boldMatch[1].length, type: "bold", match: boldMatch });
      if (italicMatch) candidates.push({ idx: italicMatch[1].length, type: "italic", match: italicMatch });
      if (codeMatch) candidates.push({ idx: codeMatch[1].length, type: "code", match: codeMatch });
      if (linkMatch) candidates.push({ idx: linkMatch[1].length, type: "link", match: linkMatch });

      if (candidates.length === 0) {
        parts.push(remaining);
        break;
      }

      candidates.sort((a, b) => a.idx - b.idx);
      const best = candidates[0];

      if (best.match[1]) parts.push(best.match[1]);
      const subKey = `${key}-${pIdx++}`;
      if (best.type === "bold") {
        parts.push(<strong key={subKey}>{best.match[2]}</strong>);
        remaining = best.match[3];
      } else if (best.type === "italic") {
        parts.push(<em key={subKey}>{best.match[2]}</em>);
        remaining = best.match[4];
      } else if (best.type === "code") {
        parts.push(<code key={subKey} className="rounded bg-muted px-1 font-mono text-[0.85em]">{best.match[2]}</code>);
        remaining = best.match[3];
      } else if (best.type === "link") {
        parts.push(
          <a key={subKey} href={best.match[3]} target="_blank" rel="noopener noreferrer" className="text-primary underline">
            {best.match[2]}
          </a>
        );
        remaining = best.match[4];
      }
    }
    return <>{parts}</>;
  };

  while (i < lines.length) {
    const line = lines[i];

    // Code fence
    if (line.startsWith("```")) {
      const fenceLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) {
        fenceLines.push(lines[i]);
        i++;
      }
      i++; // skip closing ```
      nodes.push(
        <pre key={`fence-${i}`} className="my-2 overflow-x-auto rounded bg-muted p-3 font-mono text-xs">
          <code>{fenceLines.join("\n")}</code>
        </pre>
      );
      continue;
    }

    // Headings
    if (line.startsWith("### ")) {
      nodes.push(<h3 key={`h3-${i}`} className="mt-3 mb-1 text-sm font-semibold">{renderInline(line.slice(4), `h3-${i}`)}</h3>);
      i++;
      continue;
    }
    if (line.startsWith("## ")) {
      nodes.push(<h2 key={`h2-${i}`} className="mt-4 mb-1 text-base font-semibold">{renderInline(line.slice(3), `h2-${i}`)}</h2>);
      i++;
      continue;
    }
    if (line.startsWith("# ")) {
      nodes.push(<h1 key={`h1-${i}`} className="mt-2 mb-2 text-lg font-bold">{renderInline(line.slice(2), `h1-${i}`)}</h1>);
      i++;
      continue;
    }

    // Unordered list
    if (line.startsWith("- ") || line.startsWith("* ")) {
      const listItems: React.ReactNode[] = [];
      while (i < lines.length && (lines[i].startsWith("- ") || lines[i].startsWith("* "))) {
        listItems.push(
          <li key={`li-${i}`} className="ml-4 list-disc">{renderInline(lines[i].slice(2), `li-${i}`)}</li>
        );
        i++;
      }
      nodes.push(<ul key={`ul-${i}`} className="my-1 space-y-0.5 text-sm">{listItems}</ul>);
      continue;
    }

    // Empty line → paragraph break (skip)
    if (line.trim() === "") {
      i++;
      continue;
    }

    // Paragraph
    nodes.push(
      <p key={`p-${i}`} className="my-1 text-sm leading-relaxed">
        {renderInline(line, `p-${i}`)}
      </p>
    );
    i++;
  }

  return nodes;
}

export function MarkdownPanelViz({ panel }: Props) {
  const markdown = (panel.config.markdown as string | undefined) ?? "";

  if (!markdown.trim()) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
        No content.
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto px-3 py-2">
      {renderMarkdown(markdown)}
    </div>
  );
}
