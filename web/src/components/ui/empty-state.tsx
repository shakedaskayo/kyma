import * as React from "react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Standard empty state — the icon-in-halo + headline + description + optional
 * action pattern used across list/grid pages. Codifies what was hand-rolled in
 * Data Sources / Credentials / Agent / Query / Dashboards.
 */
export interface EmptyStateProps extends React.HTMLAttributes<HTMLDivElement> {
  icon?: LucideIcon;
  title: string;
  description?: string;
  action?: React.ReactNode;
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  className,
  ...props
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center px-6 py-16 text-center",
        className,
      )}
      {...props}
    >
      {Icon && (
        <div className="mb-4 rounded-2xl bg-primary/10 p-4 text-primary ring-1 ring-inset ring-primary/15">
          <Icon className="h-8 w-8" />
        </div>
      )}
      <h2 className="text-base font-semibold text-foreground">{title}</h2>
      {description && (
        <p className="mt-1 max-w-sm text-sm text-muted-foreground">{description}</p>
      )}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}
