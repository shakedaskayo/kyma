/**
 * PensieveExplore — the unified data-exploration surface.
 *
 * One smart input that auto-detects keyword search vs raw KQL/SQL and routes to
 * the matching engine, sharing a single shell: scope + time-range + Run, a
 * results area, and a row-detail drawer.
 *
 *   keyword → usePensieveSearch (/v1/search): instant hybrid (lexical + vector,
 *             RRF-fused) ranked hits across all sources in scope, with db.table
 *             provenance. Click a hit to inspect; switch to KQL/SQL to correlate.
 *   kql/sql → usePensieveQuery (/v1/query): schema browser → field-stats rail,
 *             timestamp histogram, results grid, chart, row drawer.
 *
 * The detected mode is a clickable badge so auto-detection is never a mystery.
 */

import React, { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Play, Square, Search as SearchIcon, Code2, Network, Brain, GitFork } from "lucide-react";
import { autoChartAxes } from "@pensieve-ai/client";
import type { Column, HybridSearchHit, SchemaDoc } from "@pensieve-ai/client";

import { PensieveErrorBoundary } from "../internal/PensieveErrorBoundary";
import { Button } from "../internal/ui/button";
import { PensieveContext, usePensieveClient, usePensieveContext } from "../provider/context";
import { cn } from "../internal/cn";

import { usePensieveQuery } from "../hooks/usePensieveQuery";
import { usePensieveSearch } from "../hooks/usePensieveSearch";
import { RowDetailDrawer } from "../discover/RowDetailDrawer";
import { serializePills } from "../discover/discoverGrammar";
import { resolveTimeRange } from "../discover/useDiscoverSearch";
import { formatCell } from "../discover/columns";
import { colorForKey } from "../internal/data-palette";
import type { Pill, Scope } from "../discover/types";

import { SchemaBrowser } from "../query/schema/SchemaBrowser";
import { ResultsGrid } from "../query/results/ResultsGrid";
import { RowFilter } from "../query/results/RowFilter";
import { ChartPanel } from "../query/chart/ChartPanel";
import { HistogramTimeline } from "../discover/HistogramTimeline";
import { FieldStats } from "../discover/FieldStats";
import { TimeRangePicker } from "../query/time-range/TimeRangePicker";
import { prependTimeFilter } from "../query/time-range/time-range";
import type { TimeRange } from "../query/time-range/time-range-types";

import { detectMode, modeLabel, type ExploreMode } from "./detectMode";

export interface PensieveExploreProps {
  /** Initial input text. */
  defaultQuery?: string;
  /** Initial time range. Defaults to "all time" so any data shows on first run. */
  timeRange?: TimeRange;
  /**
   * Database scope. "*" (default) searches/queries across all databases; a
   * concrete name filters. Drives `x-database` for KQL/SQL and the source glob
   * for keyword search.
   */
  database?: string;
  className?: string;
  style?: React.CSSProperties;
  fallback?: React.ReactNode;
  /**
   * Run a search/query automatically on mount so the surface lands with data
   * instead of an empty state — re-running the (persisted) `defaultQuery`, or,
   * when it's empty, an empty keyword search that returns the most recent rows
   * across the sources in scope. Defaults to `true`. Set `false` for embedders
   * that don't want an automatic request on mount.
   */
  autoRun?: boolean;
  onQueryChange?: (q: string) => void;
  /**
   * Called when the user clicks "View in graph" on a result that is a graph
   * entity (a row from a `*_nodes` / file-candidate table with an `id`). The
   * consumer deep-links to the graph page focused on that node.
   */
  onViewInGraph?: (nodeId: string, db: string) => void;
}

function PensieveExploreInner({
  defaultQuery = "",
  timeRange: timeRangeProp,
  database = "*",
  className,
  style,
  autoRun = true,
  onQueryChange,
  onViewInGraph,
}: Omit<PensieveExploreProps, "fallback">) {
  const client = usePensieveClient();
  const endpoint = client.transport.endpoint;

  const [input, setInput] = useState(defaultQuery);
  const [forcedMode, setForcedMode] = useState<ExploreMode | null>(null);
  const [timeRange, setTimeRange] = useState<TimeRange>(timeRangeProp ?? { preset: "none" });
  const [rowFilter, setRowFilter] = useState("");
  const [openRow, setOpenRow] = useState<{ source: string; row: Record<string, unknown> } | null>(null);
  const [submittedMode, setSubmittedMode] = useState<ExploreMode | null>(null);
  // Search target: which backend store to search against.
  const [searchTarget, setSearchTarget] = useState<"data" | "memory" | "graph">("data");
  // Selected columns for the search document table (Kibana-style field pinning).
  const [cols, setCols] = useState<string[]>([]);

  const { data: schema } = useQuery<SchemaDoc>({
    queryKey: ["pensieve", endpoint, "explore-schema"],
    queryFn: () => client.catalog.fetchSchema(),
    staleTime: 5 * 60_000,
  });
  const tableNames = useMemo(
    () => (schema?.databases ?? []).flatMap((db) => db.tables.map((t) => t.name)),
    [schema],
  );

  const liveMode: ExploreMode = forcedMode ?? detectMode(input, tableNames);

  const scope: Scope = useMemo(
    () => (database && database !== "*" ? { kind: "sources", sources: [`${database}.*`] } : { kind: "all" }),
    [database],
  );

  const search = usePensieveSearch();
  const { columns: qCols, rows: qRows, isRunning: qRunning, error: qError, execute, cancel: cancelQuery } =
    usePensieveQuery();

  const inputRef = useRef(input);
  inputRef.current = input;

  const run = () => {
    const text = inputRef.current;
    const mode = forcedMode ?? detectMode(text, tableNames);
    setRowFilter("");
    setOpenRow(null);
    setSubmittedMode(mode);
    if (mode === "search") {
      void search.run({
        query: text,
        scope,
        time_range: resolveTimeRange(timeRange),
        limit: 100,
        // Only set mode when it deviates from the default so the server
        // preserves the pre-existing data-only behaviour when omitted.
        ...(searchTarget !== "data" ? { mode: searchTarget } : {}),
      });
    } else {
      const effective = mode === "kql" ? prependTimeFilter(text, timeRange, schema) : text;
      void execute({ database, query: effective, language: mode }).catch(() => {});
    }
  };

  const cancel = () => cancelQuery();

  // Auto-run on mount so the page lands with data instead of the empty state:
  // re-run the persisted input, or — when it's empty — an empty keyword search
  // that returns the most recent rows across the sources in scope. A non-empty
  // input waits for the schema so a bare table name is classified as KQL (not a
  // keyword search); an empty input always detects as "search" and runs at once.
  // The instance is keyed per tab + scope upstream, so this fires once per tab
  // activation / scope change.
  const autoRan = useRef(false);
  useEffect(() => {
    if (!autoRun || autoRan.current) return;
    if (inputRef.current.trim() && !schema) return;
    autoRan.current = true;
    run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoRun, schema]);

  const setInputAndNotify = (v: string) => {
    setInput(v);
    onQueryChange?.(v);
  };
  const appendToInput = (text: string) => {
    setInputAndNotify(input.trim() ? `${input.trim()} ${text}` : text);
  };

  const running = search.isRunning || qRunning;
  const showQuery = submittedMode === "kql" || submittedMode === "sql";

  return (
    <div
      className={cn(
        "pv-flex pv-h-full pv-flex-col pv-overflow-hidden pv-bg-background pv-text-foreground",
        className,
      )}
      style={style}
    >
      {/* ── Smart input bar ── */}
      <div className="pv-flex pv-items-start pv-gap-2 pv-border-b pv-p-3">
        <button
          type="button"
          title="Detected mode — click to force the other mode (click again for auto)"
          onClick={() => setForcedMode(forcedMode ? null : liveMode === "search" ? "kql" : "search")}
          className={cn(
            "pv-mt-0.5 pv-flex pv-shrink-0 pv-items-center pv-gap-1 pv-rounded pv-border pv-px-2 pv-py-1 pv-text-[11px] pv-font-medium",
            liveMode === "search" ? "pv-text-muted-foreground" : "pv-border-primary/40 pv-text-primary",
            forcedMode && "pv-ring-1 pv-ring-primary/40",
          )}
        >
          {liveMode === "search" ? <SearchIcon className="pv-h-3 pv-w-3" /> : <Code2 className="pv-h-3 pv-w-3" />}
          {modeLabel(liveMode)}
        </button>
        <textarea
          rows={1}
          value={input}
          spellCheck={false}
          placeholder='Search keywords (e.g. auth service:payments) — or type KQL/SQL (e.g. claude_code_events | take 50)'
          onChange={(e) => setInputAndNotify(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey || liveMode === "search")) {
              e.preventDefault();
              run();
            }
          }}
          className="pv-min-h-[36px] pv-max-h-40 pv-flex-1 pv-resize-y pv-rounded-md pv-border pv-bg-background pv-px-3 pv-py-2 pv-font-mono pv-text-xs focus:pv-outline-none focus:pv-ring-1 focus:pv-ring-primary/40"
        />
        <TimeRangePicker value={timeRange} onChange={setTimeRange} />
        {running ? (
          <Button size="sm" variant="destructive" onClick={cancel}>
            <Square className="pv-mr-1 pv-h-3.5 pv-w-3.5" /> Cancel
          </Button>
        ) : (
          <Button size="sm" onClick={run}>
            <Play className="pv-mr-1 pv-h-3.5 pv-w-3.5" /> Run
            <kbd className="pv-ml-2 pv-rounded pv-bg-primary-foreground/20 pv-px-1 pv-text-[10px]">⌘↵</kbd>
          </Button>
        )}
      </div>

      {/* ── Search-target chips (visible only in search mode) ── */}
      {liveMode === "search" && (
        <div className="pv-flex pv-items-center pv-gap-1 pv-border-b pv-px-3 pv-py-1.5">
          <span className="pv-mr-1 pv-text-[10px] pv-font-medium pv-uppercase pv-tracking-wider pv-text-muted-foreground">
            Search
          </span>
          {(["data", "memory", "graph"] as const).map((t) => {
            const Icon = t === "data" ? SearchIcon : t === "memory" ? Brain : GitFork;
            const label = t === "data" ? "Data" : t === "memory" ? "Memory" : "Graph";
            const active = searchTarget === t;
            return (
              <button
                key={t}
                type="button"
                onClick={() => setSearchTarget(t)}
                className={cn(
                  "pv-inline-flex pv-items-center pv-gap-1 pv-rounded pv-border pv-px-2 pv-py-0.5 pv-text-[11px] pv-font-medium pv-transition-colors",
                  active
                    ? "pv-border-primary/60 pv-bg-primary/10 pv-text-primary"
                    : "pv-border-transparent pv-text-muted-foreground hover:pv-border-border hover:pv-text-foreground",
                )}
                title={`Search ${label}`}
              >
                <Icon className="pv-h-3 pv-w-3" />
                {label}
              </button>
            );
          })}
        </div>
      )}

      {/* ── Body ── */}
      {submittedMode === null ? (
        <EmptyState />
      ) : showQuery ? (
        <QueryBody
          schema={schema}
          columns={qCols}
          rows={qRows}
          isRunning={qRunning}
          error={qError}
          rowFilter={rowFilter}
          onRowFilter={setRowFilter}
          onInsert={appendToInput}
          onReplaceAndRun={(kql) => {
            setForcedMode("kql");
            setInputAndNotify(kql);
            setSubmittedMode("kql");
            void execute({ database, query: prependTimeFilter(kql, timeRange, schema), language: "kql" }).catch(() => {});
          }}
          onOpenRow={(row) => setOpenRow({ source: String(row["__database"] ?? "result"), row })}
        />
      ) : (
        <SearchResults
          hits={search.hits}
          sourcesSearched={search.sourcesSearched}
          elapsedMs={search.elapsedMs}
          ran={search.ran}
          isRunning={search.isRunning}
          error={search.error}
          cols={cols}
          onToggleCol={(f) => setCols((c) => (c.includes(f) ? c.filter((x) => x !== f) : [...c, f]))}
          onAddFilter={(field, value) =>
            appendToInput(/\s/.test(value) ? `${field}:"${value}"` : `${field}:${value}`)
          }
          onZoom={(from, to) => {
            setTimeRange({ preset: "custom", from, to });
            void search.run({
              query: inputRef.current,
              scope,
              time_range: { from, to },
              limit: 100,
              ...(searchTarget !== "data" ? { mode: searchTarget } : {}),
            });
          }}
          onOpenRow={(source, row) => setOpenRow({ source, row })}
          onViewInGraph={onViewInGraph}
        />
      )}

      <RowDetailDrawer
        source={openRow?.source ?? null}
        row={openRow?.row ?? null}
        onClose={() => setOpenRow(null)}
        onAddPill={(p: Pill) => {
          appendToInput(serializePills([p]));
          setOpenRow(null);
        }}
      />
    </div>
  );
}

// ── Keyword (hybrid search) body: Kibana-style Discover ───────────────────────
//
// Fields rail · time histogram · expandable document table. Composes the hybrid
// search hits into the familiar observability-console layout.

type FieldKind = "time" | "number" | "bool" | "text" | "json" | "null";

const HIDDEN_FIELD = (k: string) => k.startsWith("__");
const TIME_NAMES = ["ts", "timestamp", "at", "time", "event_time", "observed_time", "_time", "created_at", "date"];
const ISO_RE = /^\d{4}-\d\d-\d\dT\d\d:\d\d/;

function fieldKind(v: unknown): FieldKind {
  if (v == null) return "null";
  if (typeof v === "number") return "number";
  if (typeof v === "boolean") return "bool";
  if (typeof v === "string") return ISO_RE.test(v) ? "time" : "text";
  return "json";
}

const KIND_GLYPH: Record<FieldKind, string> = {
  time: "◷",
  number: "#",
  bool: "⊤",
  text: "T",
  json: "{}",
  null: "∅",
};

function aggregateFields(hits: HybridSearchHit[]): { name: string; kind: FieldKind; count: number }[] {
  const m = new Map<string, { kind: FieldKind; count: number }>();
  for (const h of hits) {
    if (!h.row) continue;
    for (const [k, v] of Object.entries(h.row)) {
      if (HIDDEN_FIELD(k)) continue;
      const cur = m.get(k);
      const kind = fieldKind(v);
      if (!cur) m.set(k, { kind, count: 1 });
      else {
        cur.count += 1;
        if (cur.kind === "null" && kind !== "null") cur.kind = kind;
      }
    }
  }
  return Array.from(m.entries())
    .map(([name, { kind, count }]) => ({ name, kind, count }))
    .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

function detectTimeField(hits: HybridSearchHit[]): string | null {
  const sample = hits.slice(0, 8).filter((h) => !!h.row);
  for (const name of TIME_NAMES) {
    if (sample.some((h) => typeof h.row![name] === "string" && ISO_RE.test(h.row![name] as string))) return name;
  }
  for (const h of sample) {
    for (const [k, v] of Object.entries(h.row!)) {
      if (!HIDDEN_FIELD(k) && typeof v === "string" && ISO_RE.test(v)) return k;
    }
  }
  return null;
}

/** Top values of a field across hits, for the field popover. */
function topValues(hits: HybridSearchHit[], field: string, n = 5): { value: string; pct: number }[] {
  const counts = new Map<string, number>();
  let total = 0;
  for (const h of hits) {
    if (!h.row) continue;
    const v = h.row[field];
    if (v == null || v === "") continue;
    const s = formatCell(v);
    counts.set(s, (counts.get(s) ?? 0) + 1);
    total += 1;
  }
  if (total === 0) return [];
  return Array.from(counts.entries())
    .sort((a, b) => b[1] - a[1])
    .slice(0, n)
    .map(([value, c]) => ({ value, pct: Math.round((c / total) * 100) }));
}

function SourceBadge({ source }: { source: string }) {
  return (
    <span
      className="pv-inline-flex pv-shrink-0 pv-items-center pv-gap-1 pv-rounded pv-px-1.5 pv-py-0.5 pv-font-mono pv-text-[10px]"
      style={{ background: `${colorForKey(source)}1a`, color: colorForKey(source) }}
      title={source}
    >
      <span className="pv-h-1.5 pv-w-1.5 pv-rounded-full" style={{ background: colorForKey(source) }} />
      {source}
    </span>
  );
}

function FieldRow(props: {
  name: string;
  kind: FieldKind;
  selected: boolean;
  hits: HybridSearchHit[];
  onToggleCol: () => void;
  onAddFilter: (field: string, value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const top = open ? topValues(props.hits, props.name) : [];
  return (
    <li className="pv-border-b pv-border-border/40">
      <div className="pv-group pv-flex pv-items-center pv-gap-1.5 pv-px-2 pv-py-1 hover:pv-bg-accent/40">
        <span
          title={props.kind}
          className="pv-flex pv-h-4 pv-w-4 pv-shrink-0 pv-items-center pv-justify-center pv-rounded pv-bg-muted pv-text-[9px] pv-font-mono pv-text-muted-foreground"
        >
          {KIND_GLYPH[props.kind]}
        </span>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="pv-min-w-0 pv-flex-1 pv-truncate pv-text-left pv-font-mono pv-text-[11px]"
          title={props.name}
        >
          {props.name}
        </button>
        <button
          type="button"
          title={props.selected ? "Remove column" : "Add as column"}
          onClick={props.onToggleCol}
          className={cn(
            "pv-shrink-0 pv-rounded pv-px-1 pv-text-[11px] pv-opacity-0 group-hover:pv-opacity-100",
            props.selected ? "pv-text-primary pv-opacity-100" : "pv-text-muted-foreground hover:pv-text-foreground",
          )}
        >
          {props.selected ? "✓" : "＋"}
        </button>
      </div>
      {open && (
        <div className="pv-space-y-1 pv-bg-muted/30 pv-px-2 pv-py-1.5">
          {top.length === 0 ? (
            <div className="pv-text-[10px] pv-text-muted-foreground">No values in results.</div>
          ) : (
            top.map((t) => (
              <button
                key={t.value}
                type="button"
                onClick={() => props.onAddFilter(props.name, t.value)}
                title={`Filter ${props.name} = ${t.value}`}
                className="pv-flex pv-w-full pv-items-center pv-gap-2 pv-text-left"
              >
                <span className="pv-min-w-0 pv-flex-1 pv-truncate pv-font-mono pv-text-[10px]">{t.value}</span>
                <span className="pv-shrink-0 pv-tabular-nums pv-text-[10px] pv-text-muted-foreground">{t.pct}%</span>
                <span className="pv-h-1 pv-w-8 pv-shrink-0 pv-overflow-hidden pv-rounded pv-bg-muted">
                  <span className="pv-block pv-h-full pv-bg-primary/60" style={{ width: `${t.pct}%` }} />
                </span>
              </button>
            ))
          )}
        </div>
      )}
    </li>
  );
}

// ── Memory / graph hit card (no `row`) ───────────────────────────────────────

const KIND_ICON: Record<string, React.ReactNode> = {
  memory: <Brain className="pv-h-3.5 pv-w-3.5" />,
  node: <GitFork className="pv-h-3.5 pv-w-3.5" />,
  edge: <Network className="pv-h-3.5 pv-w-3.5" />,
};

function EnrichedHitCard({
  hit,
  onViewInGraph,
}: {
  hit: HybridSearchHit;
  onViewInGraph?: (nodeId: string, db: string) => void;
}) {
  const kindIcon = hit.kind ? KIND_ICON[hit.kind] ?? <SearchIcon className="pv-h-3.5 pv-w-3.5" /> : null;
  const dot = hit.source.indexOf(".");
  const db = dot >= 0 ? hit.source.slice(0, dot) : hit.source;
  const canViewInGraph = hit.kind === "node" && hit.id && onViewInGraph;

  return (
    <li className="pv-rounded-md pv-border pv-border-border/60 pv-bg-card pv-p-3 hover:pv-border-border pv-transition-colors">
      <div className="pv-flex pv-items-start pv-gap-2">
        {/* Kind icon */}
        <span className="pv-mt-0.5 pv-flex pv-shrink-0 pv-items-center pv-justify-center pv-rounded pv-bg-muted pv-p-1 pv-text-muted-foreground">
          {kindIcon}
        </span>
        <div className="pv-min-w-0 pv-flex-1 pv-space-y-1">
          {/* Title / ID */}
          <div className="pv-flex pv-items-center pv-gap-2">
            <span className="pv-truncate pv-text-sm pv-font-medium pv-text-foreground">
              {hit.title ?? hit.id ?? hit.source}
            </span>
            {hit.memory_type && (
              <span className="pv-shrink-0 pv-rounded pv-bg-primary/10 pv-px-1.5 pv-py-0.5 pv-text-[10px] pv-font-medium pv-text-primary">
                {hit.memory_type}
              </span>
            )}
          </div>
          {/* Content preview */}
          {hit.content_preview && (
            <p className="pv-line-clamp-3 pv-text-xs pv-text-muted-foreground">{hit.content_preview}</p>
          )}
          {/* Footer: source + score + actions */}
          <div className="pv-flex pv-items-center pv-gap-2 pv-pt-0.5">
            <SourceBadge source={hit.source} />
            {hit.id && (
              <span className="pv-font-mono pv-text-[10px] pv-text-muted-foreground" title="Entity ID">
                {hit.id.length > 20 ? `${hit.id.slice(0, 20)}…` : hit.id}
              </span>
            )}
            <span className="pv-ml-auto pv-tabular-nums pv-text-[10px] pv-text-muted-foreground">
              {hit.score.toFixed(3)}
            </span>
            {canViewInGraph && (
              <button
                type="button"
                onClick={() => onViewInGraph!(hit.id!, db)}
                className="pv-inline-flex pv-items-center pv-gap-1 pv-rounded pv-border pv-border-primary/40 pv-px-1.5 pv-py-0.5 pv-text-[10px] pv-text-primary hover:pv-bg-primary/10"
                title="Open in graph"
              >
                <Network className="pv-h-3 pv-w-3" /> View in graph
              </button>
            )}
          </div>
        </div>
      </div>
    </li>
  );
}

function SearchResults(props: {
  hits: HybridSearchHit[];
  sourcesSearched: number;
  elapsedMs: number;
  ran: boolean;
  isRunning: boolean;
  error: unknown;
  cols: string[];
  onToggleCol: (field: string) => void;
  onAddFilter: (field: string, value: string) => void;
  onZoom: (from: string, to: string) => void;
  onOpenRow: (source: string, row: Record<string, unknown>) => void;
  onViewInGraph?: (nodeId: string, db: string) => void;
}) {
  const { hits, cols, isRunning, error, ran } = props;
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  // Partition hits: data hits have a `row`, memory/graph hits do not.
  const dataHits = useMemo(() => hits.filter((h) => !!h.row), [hits]);
  const enrichedHits = useMemo(() => hits.filter((h) => !h.row), [hits]);
  const isEnrichedMode = enrichedHits.length > 0 && dataHits.length === 0;
  const fields = useMemo(() => aggregateFields(hits), [hits]);
  const timeField = useMemo(() => detectTimeField(hits), [hits]);
  // Only use rows that have data (for histogram + table).
  const rows = useMemo(() => dataHits.map((h) => h.row!), [dataHits]);

  if (isRunning) return <Centered>Searching…</Centered>;
  if (error != null) {
    return (
      <div
        role="alert"
        className="pv-m-3 pv-rounded pv-border pv-border-destructive/50 pv-bg-destructive/10 pv-p-3 pv-text-xs pv-text-destructive"
      >
        <span className="pv-font-semibold">Search failed:</span> {formatErr(error)}
      </div>
    );
  }
  if (ran && hits.length === 0) {
    return <Centered>No matches. Try different keywords or widen the time range.</Centered>;
  }

  const toggleExpand = (i: number) =>
    setExpanded((s) => {
      const n = new Set(s);
      n.has(i) ? n.delete(i) : n.add(i);
      return n;
    });

  // Memory/graph mode: card list (no `row`)
  if (isEnrichedMode) {
    return (
      <div className="pv-flex pv-min-h-0 pv-flex-1 pv-flex-col">
        <div className="pv-flex pv-items-center pv-gap-3 pv-border-b pv-px-3 pv-py-1 pv-text-[11px] pv-text-muted-foreground">
          <span className="pv-font-semibold pv-text-foreground">{hits.length.toLocaleString()}</span> hits
          <span>·</span>
          <span>{props.sourcesSearched} sources</span>
          <span className="pv-ml-auto pv-tabular-nums">{props.elapsedMs} ms</span>
        </div>
        <div className="pv-min-h-0 pv-flex-1 pv-overflow-auto pv-p-3">
          <ul className="pv-space-y-2">
            {enrichedHits.map((h, i) => (
              <EnrichedHitCard
                key={i}
                hit={h}
                onViewInGraph={props.onViewInGraph}
              />
            ))}
          </ul>
        </div>
      </div>
    );
  }

  // Data mode (default): Kibana-style fields rail + document table
  return (
    <div className="pv-flex pv-min-h-0 pv-flex-1">
      {/* Fields rail */}
      <aside className="pv-flex pv-w-56 pv-shrink-0 pv-flex-col pv-overflow-hidden pv-border-r">
        <div className="pv-border-b pv-px-2 pv-py-1.5 pv-text-[10px] pv-font-semibold pv-uppercase pv-tracking-wider pv-text-muted-foreground">
          Fields · {fields.length}
        </div>
        <ul className="pv-min-h-0 pv-flex-1 pv-overflow-auto">
          {fields.map((f) => (
            <FieldRow
              key={f.name}
              name={f.name}
              kind={f.kind}
              selected={cols.includes(f.name)}
              hits={hits}
              onToggleCol={() => props.onToggleCol(f.name)}
              onAddFilter={props.onAddFilter}
            />
          ))}
        </ul>
      </aside>

      {/* Histogram + document table */}
      <main className="pv-flex pv-min-w-0 pv-flex-1 pv-flex-col pv-min-h-0">
        <div className="pv-flex pv-items-center pv-gap-3 pv-border-b pv-px-3 pv-py-1 pv-text-[11px] pv-text-muted-foreground">
          <span className="pv-font-semibold pv-text-foreground">{hits.length.toLocaleString()}</span> hits
          <span>·</span>
          <span>{props.sourcesSearched} sources</span>
          <span className="pv-ml-auto pv-tabular-nums">{props.elapsedMs} ms</span>
        </div>
        {timeField && (
          <div className="pv-shrink-0 pv-border-b">
            <HistogramTimeline rows={rows} timeCol={timeField} onBucketClick={(from, to) => props.onZoom(from.toISOString(), to.toISOString())} />
          </div>
        )}
        <div className="pv-min-h-0 pv-flex-1 pv-overflow-auto pv-text-xs">
          <table className="pv-w-full pv-border-collapse">
            <thead className="pv-sticky pv-top-0 pv-z-10 pv-bg-background">
              <tr className="pv-border-b pv-text-left pv-text-[10px] pv-uppercase pv-tracking-wider pv-text-muted-foreground">
                <th className="pv-w-6" />
                {timeField && <th className="pv-px-2 pv-py-1 pv-font-medium">{timeField}</th>}
                {cols.length === 0 ? (
                  <th className="pv-px-2 pv-py-1 pv-font-medium">Document</th>
                ) : (
                  cols.map((c) => (
                    <th key={c} className="pv-px-2 pv-py-1 pv-font-mono pv-font-medium pv-normal-case pv-tracking-normal">
                      {c}
                    </th>
                  ))
                )}
                <th className="pv-px-2 pv-py-1 pv-font-medium">Source</th>
                <th className="pv-px-2 pv-py-1 pv-text-right pv-font-medium">_score</th>
              </tr>
            </thead>
            <tbody>
              {dataHits.map((h, i) => {
                const isOpen = expanded.has(i);
                const colSpan = 2 + (timeField ? 1 : 0) + (cols.length === 0 ? 1 : cols.length);
                return (
                  <React.Fragment key={i}>
                    <tr
                      className="pv-border-b pv-border-border/40 hover:pv-bg-accent/30 pv-cursor-pointer pv-align-top"
                      onClick={() => toggleExpand(i)}
                    >
                      <td className="pv-py-1 pv-pl-2 pv-text-muted-foreground">{isOpen ? "▾" : "▸"}</td>
                      {timeField && (
                        <td className="pv-whitespace-nowrap pv-px-2 pv-py-1 pv-font-mono pv-text-[11px] pv-text-muted-foreground">
                          {formatCell(h.row![timeField])}
                        </td>
                      )}
                      {cols.length === 0 ? (
                        <td className="pv-px-2 pv-py-1">
                          <div className="pv-flex pv-flex-wrap pv-gap-x-3 pv-gap-y-0.5 pv-font-mono pv-text-[11px]">
                            {Object.entries(h.row!)
                              .filter(([k]) => !HIDDEN_FIELD(k) && k !== timeField)
                              .slice(0, 8)
                              .map(([k, v]) => (
                                <span key={k} className="pv-truncate">
                                  <span className="pv-text-muted-foreground">{k}:</span>{" "}
                                  <span>{formatCell(v).slice(0, 120)}</span>
                                </span>
                              ))}
                          </div>
                        </td>
                      ) : (
                        cols.map((c) => (
                          <td key={c} className="pv-max-w-[28ch] pv-truncate pv-px-2 pv-py-1 pv-font-mono pv-text-[11px]">
                            {formatCell(h.row![c])}
                          </td>
                        ))
                      )}
                      <td className="pv-px-2 pv-py-1">
                        <SourceBadge source={h.source} />
                      </td>
                      <td className="pv-px-2 pv-py-1 pv-text-right pv-tabular-nums pv-text-[10px] pv-text-muted-foreground">
                        {h.score.toFixed(3)}
                      </td>
                    </tr>
                    {isOpen && (
                      <tr className="pv-border-b pv-border-border/40 pv-bg-muted/20">
                        <td />
                        <td colSpan={colSpan} className="pv-px-2 pv-py-2">
                          <div className="pv-mb-1 pv-flex pv-items-center pv-gap-2">
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation();
                                props.onOpenRow(h.source, h.row!);
                              }}
                              className="pv-rounded pv-border pv-px-1.5 pv-py-0.5 pv-text-[10px] pv-text-muted-foreground hover:pv-text-foreground"
                            >
                              Open detail
                            </button>
                            {(() => {
                              const g = graphEntityRef(h.source, h.row!);
                              if (!g || !props.onViewInGraph) return null;
                              return (
                                <button
                                  type="button"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    props.onViewInGraph!(g.id, g.db);
                                  }}
                                  className="pv-inline-flex pv-items-center pv-gap-1 pv-rounded pv-border pv-border-primary/40 pv-px-1.5 pv-py-0.5 pv-text-[10px] pv-text-primary hover:pv-bg-primary/10"
                                  title="Open this entity in the graph"
                                >
                                  <Network className="pv-h-3 pv-w-3" /> View in graph
                                </button>
                              );
                            })()}
                          </div>
                          <dl className="pv-grid pv-grid-cols-[minmax(8rem,12rem)_1fr] pv-gap-x-3 pv-gap-y-0.5 pv-font-mono pv-text-[11px]">
                            {Object.entries(h.row!)
                              .filter(([k]) => !HIDDEN_FIELD(k))
                              .map(([k, v]) => (
                                <React.Fragment key={k}>
                                  <dt className="pv-truncate pv-text-muted-foreground">{k}</dt>
                                  <dd className="pv-whitespace-pre-wrap pv-break-words">{formatCell(v)}</dd>
                                </React.Fragment>
                              ))}
                          </dl>
                        </td>
                      </tr>
                    )}
                  </React.Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      </main>
    </div>
  );
}

// ── Query (KQL/SQL) body ──────────────────────────────────────────────────────

function QueryBody(props: {
  schema?: SchemaDoc;
  columns: Column[];
  rows: Record<string, unknown>[];
  isRunning: boolean;
  error: unknown;
  rowFilter: string;
  onRowFilter: (v: string) => void;
  onInsert: (t: string) => void;
  onReplaceAndRun: (kql: string) => void;
  onOpenRow: (row: Record<string, unknown>) => void;
}) {
  const { schema, columns, rows, isRunning, error, rowFilter } = props;
  const hasResults = columns.length > 0 && rows.length > 0;
  const chartSpec = hasResults ? autoChartAxes(columns) : null;
  const isPlottable = chartSpec !== null && chartSpec.type !== "none";
  const timeColName = useMemo(() => columns.find((c) => c.kind === "time")?.name ?? null, [columns]);

  return (
    <div className="pv-flex pv-min-h-0 pv-flex-1">
      <aside className="pv-flex pv-w-60 pv-shrink-0 pv-flex-col pv-overflow-hidden pv-border-r">
        <div className="pv-min-h-0 pv-flex-1 pv-overflow-auto">
          {hasResults ? (
            <FieldStats rows={rows} columns={columns} onAddFilter={(col, value) => props.onInsert(`| where ${col} == "${value}"`)} />
          ) : (
            <SchemaBrowser schema={schema} onInsert={props.onInsert} onReplaceAndRun={(kql) => props.onReplaceAndRun(kql)} />
          )}
        </div>
      </aside>
      <main className="pv-flex pv-min-w-0 pv-flex-1 pv-flex-col pv-min-h-0">
        {error != null && !isRunning && (
          <div
            role="alert"
            className="pv-m-3 pv-rounded pv-border pv-border-destructive/50 pv-bg-destructive/10 pv-p-3 pv-text-xs pv-text-destructive"
          >
            <span className="pv-font-semibold">Query failed:</span> {formatErr(error)}
          </div>
        )}
        {hasResults && timeColName && <HistogramTimeline rows={rows} timeCol={timeColName} />}
        {isRunning && <Centered>Running…</Centered>}
        {!isRunning && error == null && !hasResults && (
          <Centered>{columns.length === 0 ? "Run a query to see results." : "0 rows returned."}</Centered>
        )}
        {hasResults && (
          <div className="pv-flex pv-min-h-0 pv-flex-1 pv-flex-col">
            <RowFilter
              value={rowFilter}
              onChange={props.onRowFilter}
              totalRows={rows.length}
              filteredRows={
                rowFilter.trim()
                  ? rows.filter((row) =>
                      columns.some((c) => {
                        const v = row[c.name];
                        return v != null && String(v).toLowerCase().includes(rowFilter.trim().toLowerCase());
                      }),
                    ).length
                  : rows.length
              }
            />
            <div className="pv-min-h-0 pv-flex-1 pv-overflow-hidden">
              <ResultsGrid columns={columns} rows={rows} filter={rowFilter} />
            </div>
            {isPlottable && (
              <div className="pv-h-44 pv-shrink-0 pv-border-t">
                <ChartPanel columns={columns} rows={rows} />
              </div>
            )}
          </div>
        )}
      </main>
    </div>
  );
}

function Centered({ children }: { children: React.ReactNode }) {
  return (
    <div className="pv-flex pv-flex-1 pv-items-center pv-justify-center pv-p-6 pv-text-xs pv-text-muted-foreground">
      {children}
    </div>
  );
}

function EmptyState() {
  return (
    <div className="pv-flex pv-flex-1 pv-items-center pv-justify-center pv-p-8 pv-text-center pv-text-sm pv-text-muted-foreground">
      <div className="pv-space-y-1">
        <div>Search your data or run a query.</div>
        <div className="pv-text-xs pv-opacity-70">
          Type keywords for instant hybrid search, or a table name / KQL / SQL to query. Press ⌘↵ to run.
        </div>
      </div>
    </div>
  );
}

/** If a hit is a graph entity (a `*_nodes` / file-candidate row with an id),
 *  return its `{db, id}` so the caller can deep-link to the graph. */
function graphEntityRef(source: string, row: Record<string, unknown>): { db: string; id: string } | null {
  const dot = source.indexOf(".");
  const db = dot >= 0 ? source.slice(0, dot) : source;
  const table = dot >= 0 ? source.slice(dot + 1) : source;
  const isNodeTable = table.endsWith("_nodes") || table === "memory_nodes" || table === "file_candidates";
  const id = row["id"];
  if (isNodeTable && typeof id === "string" && id) return { db, id };
  return null;
}

function formatErr(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

export function PensieveExplore({ fallback, database, ...rest }: PensieveExploreProps): JSX.Element {
  const ctx = usePensieveContext();
  const scopedCtx = database ? { ...ctx, client: ctx.client.withDatabase(database) } : ctx;
  return (
    <PensieveContext.Provider value={scopedCtx}>
      <PensieveErrorBoundary fallback={fallback}>
        <PensieveExploreInner database={database} {...rest} />
      </PensieveErrorBoundary>
    </PensieveContext.Provider>
  );
}
