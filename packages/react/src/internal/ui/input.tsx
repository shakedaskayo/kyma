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
          "ky-flex ky-h-9 ky-w-full ky-rounded-md ky-border ky-border-input ky-bg-background ky-px-3 ky-py-1 ky-text-sm ky-shadow-sm ky-transition-colors file:ky-border-0 file:ky-bg-transparent file:ky-text-sm file:ky-font-medium placeholder:ky-text-muted-foreground focus-visible:ky-outline-none focus-visible:ky-ring-1 focus-visible:ky-ring-ring disabled:ky-cursor-not-allowed disabled:ky-opacity-50",
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
