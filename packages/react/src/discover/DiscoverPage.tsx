/**
 * DiscoverPage — the internal orchestrator for KymaDiscover.
 *
 * Stripped from the web app version:
 * - useWorkspace / tab state → props + per-instance DiscoverStore
 * - SavedViewsMenu → removed (web re-attaches its own; embeddable has no concept)
 * - Live toggle → removed (LiveSession v1 omission; live prop reserved for v2)
 * - TimeRangePicker → uses local packages/react version
 * - toast → removed (web-app dependency); errors are rendered inline
 * - newTab (Query Editor export) → exposed as onExportKql prop (consumer decides)
 */

import { useState, useRef } from "react";
import { Button } from "../internal/ui/button";
import { TimeRangePicker } from "../query/time-range/TimeRangePicker";
import { ScopePicker } from "./ScopePicker";
import { QueryBar } from "./QueryBar";
import { SourcesRail } from "./SourcesRail";
import { FieldsRail } from "./FieldsRail";
import { Histogram } from "./Histogram";
import { StreamView } from "./StreamView";
import { SourceTableView } from "./SourceTableView";
import { SummaryLine } from "./SummaryLine";
import { RowDetailDrawer } from "./RowDetailDrawer";
import { useDiscoverSearch, resolveTimeRange } from "./useDiscoverSearch";
import { parseSearch, serializePills } from "./discoverGrammar";
import { compileToKql } from "./compileToKql";
import type { ExportSource } from "./compileToKql";
import { stringColumnsOf } from "./columns";
import { mergeSources } from "./stream";
import { createDiscoverStore, DiscoverStoreContext, useDiscoverStore } from "./discover-store";
import type { Pill, Scope } from "./types";
import type { TimeRange } from "../query/time-range/time-range-types";

const PRESET_LABEL: Record<string, string> = {
  "5m": "last 5m", "15m": "last 15m", "1h": "last 1h", "6h": "last 6h",
  "24h": "last 24h", "7d": "last 7d", "30d": "last 30d", custom: "custom range",
};

// ── Public prop surface ───────────────────────────────────────────────────────

export type DiscoverPageProps = {
  /** Initial search query string. */
  initialSearch?: string;
  /** Initial scope. Defaults to all sources. */
  initialScope?: Scope;
  /** Initial time range. Defaults to last 1 hour. */
  initialTimeRange?: TimeRange;
  /**
   * Called when the user clicks "Export to KQL". Receives the compiled KQL
   * string. The consumer decides what to do with it (e.g. open a query editor).
   */
  onExportKql?: (kql: string) => void;
  /**
   * Called when the user opens a row detail drawer. Receives the row data and
   * the source key. Added to support the KymaDiscover onRowOpen callback.
   */
  onRowOpen?: (row: Record<string, unknown>, source: string) => void;
  /**
   * Called on every search text change. Useful for external state persistence
   * (e.g. the web route persisting query per tab).
   */
  onSearchChange?: (search: string) => void;
};

// ── Internal component (reads from DiscoverStore) ─────────────────────────────

function DiscoverInner({ onExportKql, onRowOpen, onSearchChange }: {
  onExportKql?: (kql: string) => void;
  onRowOpen?: (row: Record<string, unknown>, source: string) => void;
  onSearchChange?: (search: string) => void;
}) {
  const search = useDiscoverStore((s) => s.search);
  const scope = useDiscoverStore((s) => s.scope);
  const timeRange = useDiscoverStore((s) => s.timeRange);
  const visibleSources = useDiscoverStore((s) => s.visibleSources);
  const selectedSource = useDiscoverStore((s) => s.selectedSource);
  const columns = useDiscoverStore((s) => s.columns);
  const viewMode = useDiscoverStore((s) => s.viewMode);
  const submitted = useDiscoverStore((s) => s.submitted);

  const _setSearch = useDiscoverStore((s) => s.setSearch);
  const setSearch = (v: string) => { _setSearch(v); onSearchChange?.(v); };
  const setScope = useDiscoverStore((s) => s.setScope);
  const setTimeRange = useDiscoverStore((s) => s.setTimeRange);
  const toggleVisible = useDiscoverStore((s) => s.toggleVisible);
  const setSelected = useDiscoverStore((s) => s.setSelected);
  const toggleColumn = useDiscoverStore((s) => s.toggleColumn);
  const setViewMode = useDiscoverStore((s) => s.setViewMode);
  const submit = useDiscoverStore((s) => s.submit);

  const [openRow, setOpenRow] = useState<{ source: string; row: Record<string, unknown> } | null>(null);

  const handleOpenRow = (source: string, row: Record<string, unknown>) => {
    setOpenRow({ source, row });
    onRowOpen?.(row, source);
  };

  const { results, cancel } = useDiscoverSearch({
    search: submitted,
    scope,
    timeRange,
    enabled: true,
  });

  const selected = selectedSource ? results.sources.get(selectedSource) ?? null : null;
  const streamRows = mergeSources(Array.from(results.sources.values()), visibleSources);
  const tableSrc =
    typeof viewMode === "object" ? results.sources.get(viewMode.table) ?? null : null;

  const insertFilter = (text: string) => {
    const next = search.trim() ? `${search.trim()} ${text}` : text;
    setSearch(next);
  };

  const pillToText = (p: Pill) => serializePills([p]);

  const handleExportKql = () => {
    if (!onExportKql) return;
    const tr = resolveTimeRange(timeRange);
    let pills: Pill[] = [];
    try {
      pills = parseSearch(search);
    } catch {
      // Grammar errors surfaced in QueryBar; export without filters.
    }

    const allKeys = Array.from(results.sources.keys());
    const visibleKeys = visibleSources ?? allKeys;

    let candidateKeys: string[];
    if (selectedSource && results.sources.has(selectedSource)) {
      candidateKeys = [selectedSource];
    } else {
      candidateKeys = visibleKeys.filter((k) => results.sources.has(k));
    }

    if (candidateKeys.length === 0) {
      onExportKql("");
      return;
    }

    const dbOf = (key: string) => {
      const dot = key.indexOf(".");
      return dot >= 0 ? key.slice(0, dot) : "";
    };

    const targetDb = dbOf(candidateKeys[0]);
    const inDb = candidateKeys.filter((k) => dbOf(k) === targetDb);

    const exportSources: ExportSource[] = inDb.map((key) => {
      const s = results.sources.get(key)!;
      return {
        key: s.source,
        timestampColumn: s.timestampColumn,
        stringColumns: stringColumnsOf(s.rows, s.timestampColumn),
      };
    });

    const kql = compileToKql(exportSources, pills, tr);
    onExportKql(kql);
  };

  const handleRun = () => {
    submit();
  };

  return (
    <div className="ky-flex ky-flex-col ky-h-full">
      {/* Top bar */}
      <div className="ky-flex ky-items-center ky-gap-2 ky-p-3 ky-border-b">
        <ScopePicker value={scope} onChange={setScope} />
        <QueryBar
          value={search}
          onChange={setSearch}
          onRun={handleRun}
          onCancel={cancel}
          running={results.status === "running"}
        />
        <TimeRangePicker value={timeRange} onChange={setTimeRange} />
        {onExportKql && (
          <Button variant="ghost" size="sm" onClick={handleExportKql}>
            Export to KQL
          </Button>
        )}
      </div>

      <SummaryLine
        sourcesSearched={results.sources.size}
        windowLabel={PRESET_LABEL[timeRange.preset] ?? "all time"}
        eventCount={streamRows.length}
        finishedAt={results.finishedAt ?? null}
        status={results.status}
      />

      <div className="ky-flex ky-flex-1 ky-min-h-0">
        <aside className="ky-w-60 ky-border-r ky-overflow-auto">
          <SourcesRail
            results={results}
            visible={visibleSources}
            onToggleVisible={(src) => {
              toggleVisible(src, Array.from(results.sources.keys()));
            }}
            selected={selectedSource}
            onSelect={setSelected}
            onOpenTable={(src) => setViewMode({ table: src })}
          />
          <FieldsRail
            source={selected}
            columns={columns}
            onToggleColumn={toggleColumn}
            onInsertFilter={insertFilter}
          />
        </aside>

        <main className="ky-flex-1 ky-min-h-0 ky-flex ky-flex-col">
          {tableSrc ? (
            <div className="ky-flex-1 ky-min-h-0 ky-overflow-auto">
              <SourceTableView
                src={tableSrc}
                onBack={() => setViewMode("stream")}
                onOpenRow={(row) => handleOpenRow(tableSrc.source, row)}
              />
            </div>
          ) : (
            <>
              <Histogram
                results={results}
                rangeTo={resolveTimeRange(timeRange)?.to ?? null}
                onZoom={(from, to) => setTimeRange({ preset: "custom", from, to })}
              />
              {results.topError && (
                <div className="ky-p-3 ky-m-3 ky-border ky-border-destructive ky-rounded ky-text-sm ky-text-destructive">
                  <span className="ky-font-semibold">{results.topError.code}</span>: {results.topError.message}
                </div>
              )}
              {results.status === "done" && results.sources.size === 0 ? (
                <div className="ky-flex-1 ky-flex ky-items-center ky-justify-center">
                  <div className="ky-p-6 ky-text-center ky-text-sm ky-text-muted-foreground ky-space-y-2">
                    <div>No data sources match this scope.</div>
                    {scope.kind === "all" ? (
                      <div>
                        Internal sources (agent memory) are hidden by default.{" "}
                        <button
                          type="button"
                          className="ky-underline ky-underline-offset-2 hover:ky-text-foreground"
                          onClick={() => setScope({ kind: "sources", sources: ["memory.*"] })}
                        >
                          Search internal sources
                        </button>
                      </div>
                    ) : (
                      <div>Try widening with the Scope picker.</div>
                    )}
                  </div>
                </div>
              ) : (
                <StreamView
                  rows={streamRows}
                  sources={results.sources}
                  columns={columns}
                  onOpenRow={(source, row) => handleOpenRow(source, row)}
                />
              )}
            </>
          )}
        </main>
      </div>

      <RowDetailDrawer
        source={openRow?.source ?? null}
        row={openRow?.row ?? null}
        onClose={() => setOpenRow(null)}
        onAddPill={(p) => { insertFilter(pillToText(p)); setOpenRow(null); }}
      />
    </div>
  );
}

// ── Public wrapper: creates and provides the per-instance store ───────────────

export function DiscoverPage({
  initialSearch,
  initialScope,
  initialTimeRange,
  onExportKql,
  onRowOpen,
  onSearchChange,
}: DiscoverPageProps) {
  const storeRef = useRef<ReturnType<typeof createDiscoverStore> | null>(null);
  if (!storeRef.current) {
    storeRef.current = createDiscoverStore({
      search: initialSearch,
      scope: initialScope,
      timeRange: initialTimeRange,
    });
  }

  return (
    <DiscoverStoreContext.Provider value={storeRef.current}>
      <DiscoverInner
        onExportKql={onExportKql}
        onRowOpen={onRowOpen}
        onSearchChange={onSearchChange}
      />
    </DiscoverStoreContext.Provider>
  );
}
