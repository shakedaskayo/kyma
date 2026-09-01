/**
 * Internal Badge primitive with Pensieve pv- class prefix.
 * NOT exported from the public package surface.
 */
import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../cn";

const badgeVariants = cva(
  "pv-inline-flex pv-items-center pv-rounded-md pv-border pv-px-2.5 pv-py-0.5 pv-text-xs pv-font-semibold pv-transition-colors",
  {
    variants: {
      variant: {
        default:
          "pv-border-transparent pv-bg-primary pv-text-primary-foreground",
        secondary:
          "pv-border-transparent pv-bg-secondary pv-text-secondary-foreground",
        destructive:
          "pv-border-transparent pv-bg-destructive pv-text-destructive-foreground",
        outline: "pv-text-foreground",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return (
    <div className={cn(badgeVariants({ variant }), className)} {...props} />
  );
}

export { Badge, badgeVariants };
