import { StatusPill, type StatusTone } from "@/components/ui/status";
import type { RunStatus } from "@/sdk/dreaming";

const TONE: Record<RunStatus, StatusTone> = {
  success: "ok",
  error: "error",
  running: "running",
  skipped: "idle",
};

const LABEL: Record<RunStatus, string> = {
  success: "Success",
  error: "Error",
  running: "Running",
  skipped: "Skipped",
};

/** A run's status as a StatusPill — running pulses. */
export function RunStatusBadge({ status, className }: { status: RunStatus; className?: string }) {
  return (
    <StatusPill tone={TONE[status]} pulse={status === "running"} className={className}>
      {LABEL[status]}
    </StatusPill>
  );
}
