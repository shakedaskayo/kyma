/**
 * Internal Collapsible primitive — wraps @radix-ui/react-collapsible with
 * Pensieve pv- class prefix so it is isolated from the host app's CSS.
 * NOT exported from the public package surface.
 */
import * as React from "react";
import * as CollapsiblePrimitive from "@radix-ui/react-collapsible";
import { cn } from "../cn";

const Collapsible = CollapsiblePrimitive.Root;
const CollapsibleTrigger = CollapsiblePrimitive.Trigger;

const CollapsibleContent = React.forwardRef<
  React.ComponentRef<typeof CollapsiblePrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof CollapsiblePrimitive.Content>
>(({ className, ...props }, ref) => (
  <CollapsiblePrimitive.Content
    ref={ref}
    className={cn(
      "pv-overflow-hidden data-[state=closed]:pv-animate-none data-[state=open]:pv-animate-none",
      className,
    )}
    {...props}
  />
));
CollapsibleContent.displayName = "CollapsibleContent";

export { Collapsible, CollapsibleTrigger, CollapsibleContent };
