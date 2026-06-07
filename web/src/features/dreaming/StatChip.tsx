import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

/** A compact metric chip — icon + label + tabular-nums value. */
export function StatChip({
  icon: Icon,
  label,
  value,
  className,
}: {
  icon: LucideIcon;
  label: string;
  value: number | string;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-md border bg-muted/40 px-2 py-0.5 text-2xs text-muted-foreground",
        className,
      )}
      title={label}
    >
      <Icon className="h-3 w-3 shrink-0" />
      <span className="font-medium tabular-nums text-foreground">{value}</span>
      <span>{label}</span>
    </span>
  );
}
