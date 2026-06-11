import { AlertCircle, CheckCircle2, CircleSlash, Loader2, Minus } from "lucide-react";
import { cn } from "@/lib/utils";
import type { DataSourceStatus } from "@/sdk/datasources";

// Mirrors the rounded-full status pills from app/shell.tsx.

const STYLES: Record<
  DataSourceStatus,
  { label: string; className: string; icon: typeof CheckCircle2; spin?: boolean }
> = {
  synced: {
    label: "Synced",
    icon: CheckCircle2,
    className:
      "border-emerald-300 bg-emerald-50 text-emerald-700 dark:border-emerald-700/50 dark:bg-emerald-950/40 dark:text-emerald-400",
  },
  syncing: {
    label: "Syncing",
    icon: Loader2,
    spin: true,
    className:
      "border-amber-300 bg-amber-50 text-amber-700 dark:border-amber-700/50 dark:bg-amber-950/40 dark:text-amber-400",
  },
  error: {
    label: "Error",
    icon: AlertCircle,
    className: "border-destructive/40 bg-destructive/5 text-destructive",
  },
  disabled: {
    label: "Paused",
    icon: CircleSlash,
    className:
      "border-muted-foreground/30 bg-muted text-muted-foreground",
  },
  idle: {
    label: "Idle",
    icon: Minus,
    className:
      "border-muted-foreground/30 bg-muted text-muted-foreground",
  },
};

export function StatusBadge({
  status,
  className,
}: {
  status: DataSourceStatus;
  className?: string;
}) {
  const s = STYLES[status];
  const Icon = s.icon;
  return (
    <div
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium",
        s.className,
        className,
      )}
    >
      <Icon className={cn("h-3 w-3", s.spin && "animate-spin")} />
      {s.label}
    </div>
  );
}
