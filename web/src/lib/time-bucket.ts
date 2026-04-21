/** Pick a good histogram bucket size based on the span of the dataset. */
export type BucketSize = "1m" | "5m" | "1h" | "1d";

export const BUCKET_MS: Record<BucketSize, number> = {
  "1m":  60_000,
  "5m":  5 * 60_000,
  "1h":  60 * 60_000,
  "1d":  24 * 60 * 60_000,
};

export function pickBucketSize(spanMs: number): BucketSize {
  if (spanMs <= 10 * 60_000)          return "1m";   // ≤10 min → 1-min buckets
  if (spanMs <= 2 * 60 * 60_000)      return "5m";   // ≤2 h   → 5-min buckets
  if (spanMs <= 3 * 24 * 60 * 60_000) return "1h";   // ≤3 d   → 1-h  buckets
  return "1d";                                        // else   → 1-d  buckets
}

export function bucketLabel(ts: number, size: BucketSize): string {
  const d = new Date(ts);
  const mm = String(d.getMinutes()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const mo = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"][d.getMonth()];
  if (size === "1m" || size === "5m") return `${hh}:${mm}`;
  if (size === "1h")                  return `${mo} ${dd} ${hh}:00`;
  return `${mo} ${dd}`;
}
