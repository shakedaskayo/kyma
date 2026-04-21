import type { TimeRange, TimeRangePreset } from "@/features/tabs/workspace-store";

export function presetToKqlAgo(p: TimeRangePreset): string {
  switch (p) {
    case "5m":  return "ago(5m)";
    case "15m": return "ago(15m)";
    case "1h":  return "ago(1h)";
    case "6h":  return "ago(6h)";
    case "24h": return "ago(24h)";
    case "7d":  return "ago(7d)";
    case "30d": return "ago(30d)";
    case "custom": return "";
  }
}

export function prependTimeFilter(query: string, range: TimeRange): string {
  // If the user already filtered by timestamp, respect it.
  if (/\btimestamp\s*(>|<|between)/i.test(query)) return query;

  let filter: string;
  if (range.preset === "custom" && range.from && range.to) {
    filter = `| where timestamp between (datetime(${range.from}) .. datetime(${range.to}))`;
  } else {
    const ago = presetToKqlAgo(range.preset);
    if (!ago) return query;
    filter = `| where timestamp > ${ago}`;
  }

  // Insert immediately after the leading table name (first line).
  const lines = query.split("\n");
  const firstNonEmpty = lines.findIndex((l) => l.trim().length > 0);
  if (firstNonEmpty < 0) return `${query}\n${filter}`;
  lines.splice(firstNonEmpty + 1, 0, filter);
  return lines.join("\n");
}
