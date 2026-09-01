import { Loader2 } from "lucide-react";
import { cn } from "../../internal/cn";

/** Small spinner used while a turn is submitted but no tokens have arrived. */
export function Loader({
  className,
  size = 16,
}: {
  className?: string;
  size?: number;
}) {
  return (
    <Loader2
      className={cn("pv-animate-spin pv-text-muted-foreground", className)}
      size={size}
      aria-label="Loading"
    />
  );
}
