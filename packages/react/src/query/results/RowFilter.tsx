import { useRef } from "react";
import { Search, X } from "lucide-react";

interface RowFilterProps {
  value: string;
  onChange: (value: string) => void;
  totalRows: number;
  filteredRows: number;
}

export function RowFilter({ value, onChange, totalRows, filteredRows }: RowFilterProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const isFiltering = value.trim().length > 0;
  const showCount = isFiltering;

  return (
    <div className="ky-flex ky-items-center ky-gap-2 ky-border-b ky-bg-card ky-px-3 ky-py-1.5">
      <Search className="ky-h-3.5 ky-w-3.5 ky-shrink-0 ky-text-muted-foreground" />
      <div className="ky-relative ky-flex-1">
        <input
          ref={inputRef}
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Filter rows…"
          className={[
            "ky-w-full ky-rounded ky-border-0 ky-bg-transparent ky-py-0.5 ky-text-xs",
            "placeholder:ky-text-muted-foreground/50",
            "focus-visible:ky-outline-none",
            "ky-text-foreground",
          ].join(" ")}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              onChange("");
              inputRef.current?.blur();
            }
          }}
        />
        {value && (
          <button
            onClick={() => { onChange(""); inputRef.current?.focus(); }}
            className="ky-absolute ky-right-0 ky-top-1/2 -ky-translate-y-1/2 ky-rounded ky-p-0.5 ky-text-muted-foreground hover:ky-text-foreground ky-transition-colors"
            tabIndex={-1}
            aria-label="Clear filter"
          >
            <X className="ky-h-3 ky-w-3" />
          </button>
        )}
      </div>
      {showCount && (
        <span className="ky-shrink-0 ky-tabular-nums ky-text-[10px] ky-text-muted-foreground">
          {filteredRows.toLocaleString()} of {totalRows.toLocaleString()}
        </span>
      )}
    </div>
  );
}
