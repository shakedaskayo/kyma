import { useState } from "react";
import type { TimeRange, TimeRangePreset } from "./time-range-types";
import { Button } from "../../internal/ui/button";

const PRESETS: { value: TimeRangePreset; label: string }[] = [
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
    <div className="relative">
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
        <div className="absolute z-20 mt-1 w-48 rounded-md border bg-popover p-1 shadow-md">
          {PRESETS.map((p) => (
            <button
              key={p.value}
              className="block w-full rounded px-2 py-1 text-left text-xs hover:bg-accent"
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
