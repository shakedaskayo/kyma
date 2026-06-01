import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";

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
      className={cn("animate-spin text-muted-foreground", className)}
      size={size}
      aria-label="Loading"
    />
  );
}
