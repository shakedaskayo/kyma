/**
 * KymaDiscover — public embeddable Discover component.
 *
 * Wraps DiscoverPage in a KymaErrorBoundary and a sized container that
 * accepts className/style for layout control.
 *
 * Prop decisions:
 * - defaultQuery / scope / timeRange → forwarded as initialSearch / initialScope /
 *   initialTimeRange to DiscoverPage (which seeds the per-instance store).
 * - database → NOT wired. useDiscoverSearch obtains its client from KymaProvider
 *   context (useKymaClient), which is per-provider. There is no scoped
 *   client.withDatabase() surface on KymaClient today; adding it would require
 *   either a context clone or a new provider prop. Deferred to v2 — consumers
 *   that need a specific database should mount a nested KymaProvider with a
 *   database-scoped endpoint.
 * - onRowOpen → fires when the user clicks any row to open its detail drawer.
 *   Internally the DiscoverPage sets openRow state on click; we intercept by
 *   receiving the row from DiscoverInner's onOpenRow callback. DiscoverPage
 *   exposes this as an onRowOpen prop we add here.
 * - onExportKql → passed straight through to DiscoverPage.
 * - onSearchChange → optional; called whenever the user edits the search bar.
 *   Useful for consumers (e.g. the web route) that want to persist search text
 *   per tab. Added as an additive prop on DiscoverPage as well.
 */

import { KymaErrorBoundary } from "../internal/KymaErrorBoundary";
import { DiscoverPage } from "./DiscoverPage";
import type { Scope } from "./types";
import type { TimeRange } from "../query/time-range/time-range-types";

export interface KymaDiscoverProps {
  /** Initial search text (discover grammar). */
  defaultQuery?: string;
  /** Initial scope (sources to search). */
  scope?: Scope;
  timeRange?: TimeRange;
  /**
   * Override the default Kyma database.
   *
   * NOT wired in v1 — there is no client.withDatabase() surface in KymaClient.
   * Consumers needing a specific database should mount a dedicated KymaProvider
   * with a database-scoped endpoint.
   * @reserved
   */
  database?: string;
  className?: string;
  style?: React.CSSProperties;
  /** Custom error fallback rendered by the error boundary. */
  fallback?: React.ReactNode;
  /**
   * Called when the user opens a row detail drawer.
   * Fires with the row data; the source key is available as `row.__source` if
   * the consumer needs it (DiscoverPage does not store source in the row itself,
   * so we pass it as a separate second argument).
   */
  onRowOpen?: (row: Record<string, unknown>, source: string) => void;
  /** Called when the user clicks "Export to KQL". Receives the compiled KQL. */
  onExportKql?: (kql: string) => void;
  /**
   * Called whenever the user's search text changes (each keystroke).
   * Useful for consumers that want to persist query state externally.
   */
  onSearchChange?: (search: string) => void;
  /**
   * Called when the user changes the scope.
   * Useful for consumers that want to persist scope state externally.
   */
  onScopeChange?: (scope: Scope) => void;
  /**
   * Called when the user changes the time range.
   * Useful for consumers that want to persist time range state externally.
   */
  onTimeRangeChange?: (timeRange: TimeRange) => void;
}

export function KymaDiscover({
  defaultQuery,
  scope,
  timeRange,
  // database — reserved, not wired; see JSDoc above
  className,
  style,
  fallback,
  onRowOpen,
  onExportKql,
  onSearchChange,
  onScopeChange,
  onTimeRangeChange,
}: KymaDiscoverProps) {
  return (
    <KymaErrorBoundary fallback={fallback}>
      <div
        className={className}
        style={{ position: "relative", overflow: "hidden", ...style }}
      >
        <DiscoverPage
          initialSearch={defaultQuery}
          initialScope={scope}
          initialTimeRange={timeRange}
          onExportKql={onExportKql}
          onRowOpen={onRowOpen}
          onSearchChange={onSearchChange}
          onScopeChange={onScopeChange}
          onTimeRangeChange={onTimeRangeChange}
        />
      </div>
    </KymaErrorBoundary>
  );
}
