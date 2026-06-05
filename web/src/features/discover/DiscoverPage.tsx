import { useState } from "react";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "../tabs/workspace-store";
import { TimeRangePicker } from "@/features/time-range/TimeRangePicker";
import { ScopePicker } from "./ScopePicker";
import { QueryBar } from "./QueryBar";
import { SourcesRail } from "./SourcesRail";
import { FieldsRail } from "./FieldsRail";
import { Histogram } from "./Histogram";
import { StreamView } from "./StreamView";
import { SourceTableView } from "./SourceTableView";
import { SummaryLine } from "./SummaryLine";
import { RowDetailDrawer } from "./RowDetailDrawer";
import { SavedViewsMenu } from "./SavedViewsMenu";
import { useDiscoverSearch, resolveTimeRange } from "./useDiscoverSearch";
import { parseSearch, serializePills } from "./discoverGrammar";
import { compileToKql } from "./compileToKql";
import { mergeSources } from "./stream";
import type { Pill } from "./types";

type Props = { tabId: string };

const PRESET_LABEL: Record<string, string> = {
  "5m": "last 5m", "15m": "last 15m", "1h": "last 1h", "6h": "last 6h",
  "24h": "last 24h", "7d": "last 7d", "30d": "last 30d", custom: "custom range",
};

export function DiscoverPage({ tabId }: Props) {
  const tab = useWorkspace((s) => s.tabs.find((t) => t.id === tabId));
  const patchDiscover = useWorkspace((s) => s.patchDiscover);
  const newTab = useWorkspace((s) => s.newTab);

  const [openRow, setOpenRow] = useState<{ source: string; row: Record<string, unknown> } | null>(null);
  // The search submitted to the backend — typing edits `search` freely; Run
  // (or auto-run on scope/time change) snapshots it here.
  const isDiscover = tab?.kind === "discover";
  const st = isDiscover ? tab.state : null;
  const [submitted, setSubmitted] = useState(() => st?.search ?? "");

  const { results, cancel, liveStatus } = useDiscoverSearch({
    search: submitted,
    scope: st?.scope ?? { kind: "all" },
    timeRange: st?.timeRange ?? { preset: "1h" },
    enabled: Boolean(isDiscover),
    live: st?.live ?? false,
  });

  if (!tab || tab.kind !== "discover" || !st) return null;

  const selected = st.selectedSource ? results.sources.get(st.selectedSource) ?? null : null;
  const streamRows = mergeSources(Array.from(results.sources.values()), st.visibleSources);
  const tableSrc =
    typeof st.viewMode === "object" ? results.sources.get(st.viewMode.table) ?? null : null;

  const insertFilter = (text: string) => {
    const next = st.search.trim() ? `${st.search.trim()} ${text}` : text;
    patchDiscover(tabId, { search: next });
  };

  const pillToText = (p: Pill) => serializePills([p]);

  const openInQueryEditor = () => {
    const sources = Array.from(results.sources.keys());
    const tr = resolveTimeRange(st.timeRange);
    let pills: Pill[] = [];
    try {
      pills = parseSearch(st.search);
    } catch {
      // Grammar errors are surfaced in the QueryBar; export without filters.
    }
    const kql = compileToKql(sources, pills, tr);
    newTab({
      kind: "query",
      state: {
        title: "from discover",
        query: kql,
        timeRange: st.timeRange,
        results: { kind: "idle" },
        chart: {},
        submittedQuery: null,
      },
    });
  };

  // When live is active, editing query + Enter sends session.update via
  // changing the submitted state — the hook's argsKey change triggers update().
  const handleRun = () => {
    if (st.live) {
      // In live mode, submitting also updates the submitted text so argsKey
      // changes, which causes the hook's update effect to fire.
      setSubmitted(st.search);
    } else {
      setSubmitted(st.search);
    }
  };

  const toggleLive = () => {
    const next = !st.live;
    patchDiscover(tabId, { live: next });
    if (!next) {
      // Turning off live: cancel the session and trigger a fresh one-shot run
      cancel();
    }
  };

  return (
    <div className="flex flex-col h-full">
      {/* Top bar */}
      <div className="flex items-center gap-2 p-3 border-b">
        <ScopePicker value={st.scope} onChange={(scope) => patchDiscover(tabId, { scope })} />
        <QueryBar
          value={st.search}
          onChange={(v) => patchDiscover(tabId, { search: v })}
          onRun={handleRun}
          onCancel={cancel}
          running={results.status === "running"}
        />
        <TimeRangePicker value={st.timeRange} onChange={(tr) => patchDiscover(tabId, { timeRange: tr })} />

        {/* Live toggle */}
        <Button
          variant={st.live ? "default" : "ghost"}
          size="sm"
          onClick={toggleLive}
          className={st.live ? "text-red-500 border-red-500/50" : "text-muted-foreground"}
          title={st.live ? "Live mode on — click to stop" : "Start live mode"}
        >
          {st.live ? "🔴 Live" : "Live"}
        </Button>

        <SavedViewsMenu currentScope={st.scope} />
        <Button variant="ghost" size="sm" onClick={openInQueryEditor}>
          Open in Query Editor
        </Button>
      </div>

      <SummaryLine
        sourcesSearched={results.sources.size}
        windowLabel={PRESET_LABEL[st.timeRange.preset] ?? "all time"}
        eventCount={streamRows.length}
        finishedAt={results.finishedAt ?? null}
        status={results.status}
        liveStatus={liveStatus}
      />

      <div className="flex flex-1 min-h-0">
        <aside className="w-60 border-r overflow-auto">
          <SourcesRail
            results={results}
            visible={st.visibleSources}
            onToggleVisible={(src) => {
              const cur = st.visibleSources ?? Array.from(results.sources.keys());
              const next = cur.includes(src) ? cur.filter((s) => s !== src) : [...cur, src];
              patchDiscover(tabId, { visibleSources: next });
            }}
            selected={st.selectedSource}
            onSelect={(src) => patchDiscover(tabId, { selectedSource: src })}
            onOpenTable={(src) => patchDiscover(tabId, { viewMode: { table: src } })}
          />
          <FieldsRail
            source={selected}
            columns={st.columns}
            onToggleColumn={(f) =>
              patchDiscover(tabId, {
                columns: st.columns.includes(f)
                  ? st.columns.filter((c) => c !== f)
                  : [...st.columns, f],
              })
            }
            onInsertFilter={insertFilter}
          />
        </aside>

        <main className="flex-1 overflow-auto">
          {tableSrc ? (
            <SourceTableView
              src={tableSrc}
              onBack={() => patchDiscover(tabId, { viewMode: "stream" })}
              onOpenRow={(row) => setOpenRow({ source: tableSrc.source, row })}
            />
          ) : (
            <>
              <Histogram
                results={results}
                rangeTo={resolveTimeRange(st.timeRange)?.to ?? null}
                onZoom={(from, to) =>
                  patchDiscover(tabId, { timeRange: { preset: "custom", from, to } })
                }
              />
              {results.topError && (
                <div className="p-3 m-3 border border-destructive rounded text-sm text-destructive">
                  <span className="font-semibold">{results.topError.code}</span>: {results.topError.message}
                </div>
              )}
              <StreamView
                rows={streamRows}
                sources={results.sources}
                columns={st.columns}
                onOpenRow={(source, row) => setOpenRow({ source, row })}
              />
              {results.status === "done" && results.sources.size === 0 && (
                <div className="p-6 text-center text-sm text-muted-foreground space-y-2">
                  <div>No data sources match this scope.</div>
                  {st.scope.kind === "all" ? (
                    <div>
                      Internal sources (agent memory) are hidden by default.{" "}
                      <button
                        type="button"
                        className="underline underline-offset-2 hover:text-foreground"
                        onClick={() =>
                          patchDiscover(tabId, { scope: { kind: "sources", sources: ["memory.*"] } })
                        }
                      >
                        Search internal sources
                      </button>
                    </div>
                  ) : (
                    <div>Try widening with the Scope picker.</div>
                  )}
                </div>
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
