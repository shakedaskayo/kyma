/**
 * Time-range types for the embedded query surface.
 * Decoupled from workspace-store — no Zustand, no tab state.
 */

export type TimeRangePreset =
  | "none"
  | "5m"
  | "15m"
  | "1h"
  | "6h"
  | "24h"
  | "7d"
  | "30d"
  | "custom";

export type TimeRange = {
  preset: TimeRangePreset;
  /** ISO-8601 start (only meaningful when preset === "custom"). */
  from?: string;
  /** ISO-8601 end (only meaningful when preset === "custom"). */
  to?: string;
};
