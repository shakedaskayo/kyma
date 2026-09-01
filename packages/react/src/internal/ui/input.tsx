/**
 * Internal Input primitive — mirrors shadcn/ui Input but wired to Pensieve CSS
 * custom properties (--pensieve-*) rather than the host app's shadcn tokens.
 * NOT exported from the public package surface.
 */
import * as React from "react";
import { cn } from "../cn";

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          "pv-flex pv-h-9 pv-w-full pv-rounded-md pv-border pv-border-input pv-bg-background pv-px-3 pv-py-1 pv-text-sm pv-shadow-sm pv-transition-colors file:pv-border-0 file:pv-bg-transparent file:pv-text-sm file:pv-font-medium placeholder:pv-text-muted-foreground focus-visible:pv-outline-none focus-visible:pv-ring-1 focus-visible:pv-ring-ring disabled:pv-cursor-not-allowed disabled:pv-opacity-50",
          className,
        )}
        ref={ref}
        {...props}
      />
    );
  },
);
Input.displayName = "Input";

export { Input };
