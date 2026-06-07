/**
 * Internal Badge primitive with Kyma ky- class prefix.
 * NOT exported from the public package surface.
 */
import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../cn";

const badgeVariants = cva(
  "ky-inline-flex ky-items-center ky-rounded-md ky-border ky-px-2.5 ky-py-0.5 ky-text-xs ky-font-semibold ky-transition-colors",
  {
    variants: {
      variant: {
        default:
          "ky-border-transparent ky-bg-primary ky-text-primary-foreground",
        secondary:
          "ky-border-transparent ky-bg-secondary ky-text-secondary-foreground",
        destructive:
          "ky-border-transparent ky-bg-destructive ky-text-destructive-foreground",
        outline: "ky-text-foreground",
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
