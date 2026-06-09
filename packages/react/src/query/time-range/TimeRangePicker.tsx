import { useState } from "react";
import type { TimeRange, TimeRangePreset } from "./time-range-types";
import { Button } from "../../internal/ui/button";

const PRESETS: { value: TimeRangePreset; label: string }[] = [
  { value: "none",   label: "All time" },
  { value: "5m",     label: "Last 5 min" },
  { value: "15m",    label: "Last 15 min" },
  { value: "1h",     label: "Last 1 hour" },
  { value: "6h",     label: "Last 6 hours" },
  { value: "24h",    label: "Last 24 hours" },
  { value: "7d",     label: "Last 7 days" },
  { value: "30d",    label: "Last 30 days" },
  { value: "custom", label: "Custom…" },
];

export function TimeRangePicker({
  value,
  onChange,
  disabled = false,
  disabledReason,
}: {
  value: TimeRange;
  onChange: (r: TimeRange) => void;
  /** Disable when the active table has no time column (filter wouldn't apply). */
  disabled?: boolean;
  disabledReason?: string;
}) {
  const [open, setOpen] = useState(false);
  const label = disabled
    ? "No time column"
    : PRESETS.find((p) => p.value === value.preset)?.label ?? "Last 1 hour";
  return (
    <div className="ky-relative">
      <Button
        variant="outline"
        size="sm"
        disabled={disabled}
        title={disabled ? disabledReason : undefined}
        onClick={() => !disabled && setOpen((v) => !v)}
      >
        {label}
      </Button>
      {open && (
        <div className="ky-absolute ky-z-20 ky-mt-1 ky-w-48 ky-rounded-md ky-border ky-bg-popover ky-p-1 ky-shadow-md">
          {PRESETS.map((p) => (
            <button
              key={p.value}
              className="ky-block ky-w-full ky-rounded ky-px-2 ky-py-1 ky-text-left ky-text-xs hover:ky-bg-accent"
              onClick={() => {
                onChange({ preset: p.value });
                setOpen(false);
              }}
            >
              {p.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
