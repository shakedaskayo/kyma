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
  "ky-inline-flex ky-items-center ky-justify-center ky-gap-2 ky-whitespace-nowrap ky-rounded-md ky-text-sm ky-font-medium ky-ring-offset-background ky-transition-[color,background-color,border-color,box-shadow,transform] ky-duration-150 focus-visible:ky-outline-none focus-visible:ky-ring-2 focus-visible:ky-ring-ring focus-visible:ky-ring-offset-2 active:ky-scale-[0.98] disabled:ky-pointer-events-none disabled:ky-opacity-50 [&_svg]:ky-pointer-events-none [&_svg]:ky-size-4 [&_svg]:ky-shrink-0",
  {
    variants: {
      variant: {
        default:
          "ky-bg-primary ky-text-primary-foreground ky-shadow hover:ky-bg-primary/90",
        destructive:
          "ky-bg-destructive ky-text-destructive-foreground ky-shadow-sm hover:ky-bg-destructive/90",
        outline:
          "ky-border ky-border-input ky-bg-background ky-shadow-sm hover:ky-bg-accent hover:ky-text-accent-foreground",
        secondary:
          "ky-bg-secondary ky-text-secondary-foreground ky-shadow-sm hover:ky-bg-secondary/80",
        ghost: "hover:ky-bg-accent hover:ky-text-accent-foreground",
        link: "ky-text-primary ky-underline-offset-4 hover:ky-underline",
      },
      size: {
        default: "ky-h-10 ky-px-4 ky-py-2",
        sm: "ky-h-9 ky-rounded-md ky-px-3",
        xs: "ky-h-7 ky-rounded-md ky-px-2.5 ky-text-xs [&_svg]:ky-size-3.5",
        lg: "ky-h-11 ky-rounded-md ky-px-8",
        icon: "ky-h-10 ky-w-10",
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
