/**
 * Internal Button primitive — mirrors shadcn/ui Button but wired to Pensieve CSS
 * custom properties (--pensieve-*) rather than the host app's shadcn tokens.
 * NOT exported from the public package surface.
 */
import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../cn";

const buttonVariants = cva(
  "pv-inline-flex pv-items-center pv-justify-center pv-gap-2 pv-whitespace-nowrap pv-rounded-md pv-text-sm pv-font-medium pv-ring-offset-background pv-transition-[color,background-color,border-color,box-shadow,transform] pv-duration-150 focus-visible:pv-outline-none focus-visible:pv-ring-2 focus-visible:pv-ring-ring focus-visible:pv-ring-offset-2 active:pv-scale-[0.98] disabled:pv-pointer-events-none disabled:pv-opacity-50 [&_svg]:pv-pointer-events-none [&_svg]:pv-size-4 [&_svg]:pv-shrink-0",
  {
    variants: {
      variant: {
        default:
          "pv-bg-primary pv-text-primary-foreground pv-shadow hover:pv-bg-primary/90",
        destructive:
          "pv-bg-destructive pv-text-destructive-foreground pv-shadow-sm hover:pv-bg-destructive/90",
        outline:
          "pv-border pv-border-input pv-bg-background pv-shadow-sm hover:pv-bg-accent hover:pv-text-accent-foreground",
        secondary:
          "pv-bg-secondary pv-text-secondary-foreground pv-shadow-sm hover:pv-bg-secondary/80",
        ghost: "hover:pv-bg-accent hover:pv-text-accent-foreground",
        link: "pv-text-primary pv-underline-offset-4 hover:pv-underline",
      },
      size: {
        default: "pv-h-10 pv-px-4 pv-py-2",
        sm: "pv-h-9 pv-rounded-md pv-px-3",
        xs: "pv-h-7 pv-rounded-md pv-px-2.5 pv-text-xs [&_svg]:pv-size-3.5",
        lg: "pv-h-11 pv-rounded-md pv-px-8",
        icon: "pv-h-10 pv-w-10",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        {...props}
      />
    );
  },
);
Button.displayName = "Button";

export { Button, buttonVariants };
