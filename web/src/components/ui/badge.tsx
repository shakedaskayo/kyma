import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium transition-colors",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary/15 text-foreground",
        secondary: "border-transparent bg-muted text-muted-foreground",
        outline: "border-border-strong text-foreground",
      },
      // Palette-aligned tones — keep in sync with src/lib/data-palette.ts so
      // badges, chart series, memory chips, and graph node colors agree.
      tone: {
        none: "",
        teal: "border-transparent bg-teal-500/15 text-teal-600 dark:text-teal-300",
        cyan: "border-transparent bg-sky-500/15 text-sky-600 dark:text-sky-300",
        violet: "border-transparent bg-violet-500/15 text-violet-600 dark:text-violet-300",
        amber: "border-transparent bg-amber-500/15 text-amber-600 dark:text-amber-300",
        emerald: "border-transparent bg-emerald-500/15 text-emerald-600 dark:text-emerald-300",
        rose: "border-transparent bg-rose-500/15 text-rose-600 dark:text-rose-300",
        indigo: "border-transparent bg-indigo-500/15 text-indigo-600 dark:text-indigo-300",
        slate: "border-transparent bg-slate-500/15 text-slate-600 dark:text-slate-300",
      },
    },
    defaultVariants: { variant: "default", tone: "none" },
  },
);

export type BadgeTone = NonNullable<VariantProps<typeof badgeVariants>["tone"]>;

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

export function Badge({ className, variant, tone, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant, tone }), className)} {...props} />;
}

export { badgeVariants };
