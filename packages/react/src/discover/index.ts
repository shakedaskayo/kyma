// ── Public surface: DiscoverPage (internal orchestrator for KymaDiscover) ─────
export { DiscoverPage } from "./DiscoverPage";
export type { DiscoverPageProps } from "./DiscoverPage";

// ── Types ─────────────────────────────────────────────────────────────────────
export type { Pill, Scope, SourceKey, SourceState, DiscoverResultsState } from "./types";
export type { TimeRange } from "../query/time-range/time-range-types";

// ── Utility re-exports (used by wrapper step) ─────────────────────────────────
export { resolveTimeRange } from "./useDiscoverSearch";
export { compileToKql } from "./compileToKql";
export type { ExportSource } from "./compileToKql";
export { parseSearch, serializePills } from "./discoverGrammar";
export { mergeSources } from "./stream";
export type { StreamRow } from "./stream";

// ── Sub-components (exported for embedding flexibility) ───────────────────────
export { StreamView } from "./StreamView";
export { SummaryLine, formatSummary } from "./SummaryLine";
export { Histogram } from "./Histogram";
export { HistogramTimeline } from "./HistogramTimeline";
export type { HistogramTimelineProps } from "./HistogramTimeline";
export { FieldStats } from "./FieldStats";
export type { FieldStatsProps } from "./FieldStats";
export { SourcesRail } from "./SourcesRail";
export { FieldsRail } from "./FieldsRail";
export { QueryBar } from "./QueryBar";
export { ScopePicker } from "./ScopePicker";
export { SourceTableView } from "./SourceTableView";
export { RowDetailDrawer } from "./RowDetailDrawer";
