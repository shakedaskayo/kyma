import * as React from "react";
import { cn } from "@/lib/utils";

export type StatusTone = "ok" | "running" | "error" | "idle" | "warn";

const DOT: Record<StatusTone, string> = {
  ok: "bg-emerald-500",
  running: "bg-amber-500",
  error: "bg-destructive",
  warn: "bg-amber-500",
  idle: "bg-muted-foreground",
};

const RING: Record<StatusTone, string> = {
  ok: "ring-emerald-500/30",
  running: "ring-amber-500/30",
  error: "ring-destructive/30",
  warn: "ring-amber-500/30",
  idle: "ring-muted-foreground/20",
};

const PILL: Record<StatusTone, string> = {
  ok: "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300",
  running: "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300",
  error: "border-destructive/40 bg-destructive/10 text-destructive",
  warn: "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-300",
  idle: "border-border bg-muted/40 text-muted-foreground",
};

/** A small status dot with a soft ring. `pulse` adds a heartbeat for live states. */
export function StatusDot({
  tone,
  pulse,
  className,
  title,
}: {
  tone: StatusTone;
  pulse?: boolean;
  className?: string;
  title?: string;
}) {
  return (
    <span
      title={title}
      className={cn(
        "relative inline-block h-2 w-2 rounded-full ring-2",
        DOT[tone],
        RING[tone],
        pulse && "animate-pulse",
        className,
      )}
    />
  );
}

/** A pill combining a status dot + label, used for query/connection states. */
export function StatusPill({
  tone,
  children,
  pulse,
  className,
}: {
  tone: StatusTone;
  children: React.ReactNode;
  pulse?: boolean;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-2xs tabular-nums",
        PILL[tone],
        className,
      )}
    >
      <span className={cn("h-1.5 w-1.5 rounded-full", DOT[tone], pulse && "animate-pulse")} />
      {children}
    </span>
  );
}
