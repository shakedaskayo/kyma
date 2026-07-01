import { MousePointerClick } from "lucide-react";
import { EmptyState } from "@/components/ui/empty-state";

/** Right-pane placeholder shown when no run is selected. */
export function RunDetailEmpty() {
  return (
    <div className="flex h-full items-center justify-center">
      <EmptyState
        icon={MousePointerClick}
        title="Select a run"
        description="Choose a dreaming or consolidation run from the list to see its outcome, activity, and conversation."
      />
    </div>
  );
}
