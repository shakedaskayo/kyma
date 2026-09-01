/**
 * Shared minimal markdown renderer — headings, paragraphs, bold, italic, code,
 * links, unordered lists, fenced code blocks. Zero external dependency (avoids
 * bundle bloat). Used by the dashboard markdown panel and the graph inspector.
 */
import * as React from "react";

export function renderMarkdown(md: string): React.ReactNode[] {
  const lines = md.split("\n");
  const nodes: React.ReactNode[] = [];
  let i = 0;

  const renderInline = (text: string, key: string): React.ReactNode => {
    const parts: React.ReactNode[] = [];
    let remaining = text;
    let pIdx = 0;

    while (remaining.length > 0) {
      const boldMatch = remaining.match(/^(.*?)\*\*(.+?)\*\*(.*)/s);
      const italicMatch = remaining.match(/^(.*?)(?<!\*)\*(?!\*)(.+?)(?<!\*)\*(?!\*)(.*)/s);
      const codeMatch = remaining.match(/^(.*?)`([^`]+)`(.*)/s);
      const linkMatch = remaining.match(/^(.*?)\[([^\]]+)\]\(([^)]+)\)(.*)/s);

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
        parts.push(
          <code key={subKey} className="pv-rounded pv-bg-muted pv-px-1 pv-font-mono pv-text-[0.85em]">
            {best.match[2]}
          </code>,
        );
        remaining = best.match[3];
      } else if (best.type === "link") {
        parts.push(
          <a
            key={subKey}
            href={best.match[3]}
            target="_blank"
            rel="noopener noreferrer"
            className="pv-text-primary pv-underline"
          >
            {best.match[2]}
          </a>,
        );
        remaining = best.match[4];
      }
    }
    return <>{parts}</>;
  };

  while (i < lines.length) {
    const line = lines[i];

    if (line.startsWith("```")) {
      const fenceLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) {
        fenceLines.push(lines[i]);
        i++;
      }
      i++;
      nodes.push(
        <pre
          key={`fence-${i}`}
          className="pv-my-2 pv-overflow-x-auto pv-rounded pv-bg-muted pv-p-3 pv-font-mono pv-text-xs"
        >
          <code>{fenceLines.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    if (line.startsWith("### ")) {
      nodes.push(
        <h3 key={`h3-${i}`} className="pv-mt-3 pv-mb-1 pv-text-sm pv-font-semibold">
          {renderInline(line.slice(4), `h3-${i}`)}
        </h3>,
      );
      i++;
      continue;
    }
    if (line.startsWith("## ")) {
      nodes.push(
        <h2 key={`h2-${i}`} className="pv-mt-4 pv-mb-1 pv-text-base pv-font-semibold">
          {renderInline(line.slice(3), `h2-${i}`)}
        </h2>,
      );
      i++;
      continue;
    }
    if (line.startsWith("# ")) {
      nodes.push(
        <h1 key={`h1-${i}`} className="pv-mt-2 pv-mb-2 pv-text-lg pv-font-bold">
          {renderInline(line.slice(2), `h1-${i}`)}
        </h1>,
      );
      i++;
      continue;
    }

    if (line.startsWith("- ") || line.startsWith("* ")) {
      const listItems: React.ReactNode[] = [];
      while (i < lines.length && (lines[i].startsWith("- ") || lines[i].startsWith("* "))) {
        listItems.push(
          <li key={`li-${i}`} className="pv-ml-4 pv-list-disc">
            {renderInline(lines[i].slice(2), `li-${i}`)}
          </li>,
        );
        i++;
      }
      nodes.push(
        <ul key={`ul-${i}`} className="pv-my-1 pv-space-y-0.5 pv-text-sm">
          {listItems}
        </ul>,
      );
      continue;
    }

    if (line.trim() === "") {
      i++;
      continue;
    }

    nodes.push(
      <p key={`p-${i}`} className="pv-my-1 pv-text-sm pv-leading-relaxed">
        {renderInline(line, `p-${i}`)}
      </p>,
    );
    i++;
  }

  return nodes;
}

/** Render a markdown string as React nodes. Callers control the wrapping layout. */
export function Markdown({ source }: { source: string }) {
  return <>{renderMarkdown(source)}</>;
}
