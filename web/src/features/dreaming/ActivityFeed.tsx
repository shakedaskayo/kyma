import { useEffect, useState } from "react";
import { Activity } from "lucide-react";
import { relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { ProgressActivity } from "@/sdk/dreaming";
import { activityIcon } from "./activityIcons";

/**
 * The rolling activity ring rendered newest-FIRST. The server delivers the ring
 * newest-last, so we reverse a copy. New entries fade up. When `live`, relative
 * timestamps re-tick every second.
 */
export function ActivityFeed({
  items,
  live,
  className,
}: {
  items: ProgressActivity[];
  live?: boolean;
  className?: string;
}) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [live]);

  if (!items || items.length === 0) {
    return (
      <div className="flex items-center gap-2 px-1 py-4 text-2xs text-muted-foreground">
        <Activity className="h-3.5 w-3.5" /> No activity recorded.
      </div>
    );
  }

  // Newest first — reverse a copy of the ring (server sends newest last).
  const ordered = [...items].reverse();

  return (
    <div className={cn("space-y-0.5", className)}>
      {ordered.map((a, i) => {
        const Icon = activityIcon(a.icon);
        return (
          <div
            key={`${a.ts}-${i}`}
            className={cn(
              "flex items-start gap-2 rounded-md px-1.5 py-1 text-xs",
              i === 0 && "animate-fade-up bg-violet-500/5",
            )}
          >
            <Icon
              className={cn(
                "mt-0.5 h-3.5 w-3.5 shrink-0",
                i === 0 ? "text-violet-500" : "text-muted-foreground",
              )}
            />
            <span className="min-w-0 flex-1 break-words text-foreground">{a.text}</span>
            <span className="shrink-0 whitespace-nowrap text-2xs tabular-nums text-muted-foreground">
              {relTime(a.ts, now)}
            </span>
          </div>
        );
      })}
    </div>
  );
}
