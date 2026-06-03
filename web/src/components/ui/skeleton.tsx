import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * Shimmer skeleton placeholder. Replaces ad-hoc `animate-pulse` blocks with a
 * consistent token-aware loading shape. Uses an overlay sweep so it reads as a
 * deliberate loading state rather than a flat pulse.
 */
export function Skeleton({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-md bg-muted/60",
        "after:absolute after:inset-0 after:-translate-x-full after:animate-shimmer",
        "after:bg-gradient-to-r after:from-transparent after:via-foreground/10 after:to-transparent",
        className,
      )}
      {...props}
    />
  );
}

/** A stack of skeleton lines, handy for list/table placeholders. */
export function SkeletonRows({
  rows = 5,
  className,
}: {
  rows?: number;
  className?: string;
}) {
  return (
    <div className={cn("space-y-2", className)}>
      {Array.from({ length: rows }).map((_, i) => (
        <Skeleton key={i} className="h-9 w-full" />
      ))}
    </div>
  );
}

/** A card-shaped skeleton for grid placeholders. */
export function SkeletonCard({ className }: { className?: string }) {
  return (
    <div className={cn("rounded-lg border bg-card p-5 shadow-elev-2", className)}>
      <Skeleton className="h-4 w-1/3" />
      <Skeleton className="mt-3 h-3 w-2/3" />
      <Skeleton className="mt-2 h-3 w-1/2" />
    </div>
  );
}
