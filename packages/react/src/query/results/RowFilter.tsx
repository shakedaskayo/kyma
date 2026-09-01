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
    <div className="pv-flex pv-items-center pv-gap-2 pv-border-b pv-bg-card pv-px-3 pv-py-1.5">
      <Search className="pv-h-3.5 pv-w-3.5 pv-shrink-0 pv-text-muted-foreground" />
      <div className="pv-relative pv-flex-1">
        <input
          ref={inputRef}
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Filter rows…"
          className={[
            "pv-w-full pv-rounded pv-border-0 pv-bg-transparent pv-py-0.5 pv-text-xs",
            "placeholder:pv-text-muted-foreground/50",
            "focus-visible:pv-outline-none",
            "pv-text-foreground",
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
            className="pv-absolute pv-right-0 pv-top-1/2 -pv-translate-y-1/2 pv-rounded pv-p-0.5 pv-text-muted-foreground hover:pv-text-foreground pv-transition-colors"
            tabIndex={-1}
            aria-label="Clear filter"
          >
            <X className="pv-h-3 pv-w-3" />
          </button>
        )}
      </div>
      {showCount && (
        <span className="pv-shrink-0 pv-tabular-nums pv-text-[10px] pv-text-muted-foreground">
          {filteredRows.toLocaleString()} of {totalRows.toLocaleString()}
        </span>
      )}
    </div>
  );
}
