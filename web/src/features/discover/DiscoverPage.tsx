import { useState } from "react";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "../tabs/workspace-store";
import type { TimeRange } from "../tabs/workspace-store";
import { TimeRangePicker } from "@/features/time-range/TimeRangePicker";
import { ScopePicker } from "./ScopePicker";
import { SearchBar } from "./SearchBar";
import { SourcesRail } from "./SourcesRail";
import { FieldsRail } from "./FieldsRail";
import { Histogram } from "./Histogram";
import { SourceSection } from "./SourceSection";
import { RowDetailDrawer } from "./RowDetailDrawer";
import { SavedViewsMenu } from "./SavedViewsMenu";
import { useDiscoverSearch } from "./useDiscoverSearch";
import { parseSearch, serializePills } from "./discoverGrammar";
import { compileToKql } from "./compileToKql";
import type { Pill } from "./types";

type Props = { tabId: string };

const PRESET_MINUTES: Record<string, number> = {
  "5m": 5, "15m": 15, "1h": 60, "6h": 360,
  "24h": 1440, "7d": 10080, "30d": 43200,
};

function resolveTimeRange(t: TimeRange): { from: string; to: string } | null {
  if (t.preset === "custom" && t.from && t.to) return { from: t.from, to: t.to };
  const minutes = PRESET_MINUTES[t.preset];
  if (minutes == null) return null;
  const now = Date.now();
  return {
    from: new Date(now - minutes * 60_000).toISOString(),
    to: new Date(now).toISOString(),
  };
}

export function DiscoverPage({ tabId }: Props) {
  const tab = useWorkspace((s) => s.tabs.find((t) => t.id === tabId));
  const patchDiscover = useWorkspace((s) => s.patchDiscover);
  const newTab = useWorkspace((s) => s.newTab);

  const [openRow, setOpenRow] = useState<{ source: string; row: Record<string, unknown> } | null>(null);

  // Always call hooks unconditionally — derive enable flag from kind so the
  // early-return below doesn't break the hook order.
  const isDiscover = tab?.kind === "discover";
  const st = isDiscover ? tab.state : null;

  const { results, cancel } = useDiscoverSearch({
    search: st?.search ?? "",
    scope: st?.scope ?? { kind: "all" },
    timeRange: st?.timeRange ?? { preset: "1h" },
    enabled: Boolean(isDiscover),
  });

  if (!tab || tab.kind !== "discover" || !st) return null;

  const selected = st.selectedSource ? results.sources.get(st.selectedSource) ?? null : null;

  const submit = () => {};

  const addPill = (p: Pill) => {
    patchDiscover(tabId, { search: (st.search.trim() ? st.search.trim() + " " : "") + serializePills([p]) });
  };

  const openInQueryEditor = () => {
    const sources = Array.from(results.sources.keys());
    const tr = resolveTimeRange(st.timeRange);
    let pills: Pill[] = [];
    try { pills = parseSearch(st.search); } catch { /* surfaced elsewhere */ }
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

  return (
    <div className="flex flex-col h-full">
      {/* Top bar */}
      <div className="flex flex-col gap-2 p-3 border-b">
        <div className="flex items-center gap-2">
          <ScopePicker
            value={st.scope}
            onChange={(scope) => patchDiscover(tabId, { scope })}
          />
          <SearchBar
            value={st.search}
            onChange={(v) => patchDiscover(tabId, { search: v })}
            onSubmit={submit}
            onCancel={cancel}
            running={results.status === "running"}
          />
          <TimeRangePicker
            value={st.timeRange}
            onChange={(tr) => patchDiscover(tabId, { timeRange: tr })}
          />
          <SavedViewsMenu currentScope={st.scope} />
          <Button variant="ghost" size="sm" onClick={openInQueryEditor}>
            Open in Query Editor
          </Button>
        </div>
      </div>

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
            columns={st.columns ?? []}
            onToggleColumn={(f) =>
              patchDiscover(tabId, {
                columns: st.columns.includes(f)
                  ? st.columns.filter((c) => c !== f)
                  : [...st.columns, f],
              })
            }
            onInsertFilter={(text) =>
              patchDiscover(tabId, { search: (st.search.trim() ? st.search.trim() + " " : "") + text })
            }
          />
        </aside>

        <main className="flex-1 overflow-auto">
          <Histogram results={results} />
          {results.topError && (
            <div className="p-3 m-3 border border-destructive rounded text-sm text-destructive">
              <span className="font-semibold">{results.topError.code}</span>: {results.topError.message}
            </div>
          )}
          {Array.from(results.sources.values())
            .filter((s) => st.visibleSources == null || st.visibleSources.includes(s.source))
            .map((s) => (
              <SourceSection
                key={s.source}
                src={s}
                timeRangeActive={resolveTimeRange(st.timeRange) != null}
                onOpenRow={(row) => setOpenRow({ source: s.source, row })}
              />
            ))}
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
                      patchDiscover(tabId, {
                        scope: { kind: "sources", sources: ["memory.*"] },
                      })
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
        </main>
      </div>

      <RowDetailDrawer
        source={openRow?.source ?? null}
        row={openRow?.row ?? null}
        onClose={() => setOpenRow(null)}
        onAddPill={(p) => { addPill(p); setOpenRow(null); }}
      />
    </div>
  );
}
