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
    <div className="flex items-center gap-2 border-b bg-card px-3 py-1.5">
      <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <div className="relative flex-1">
        <input
          ref={inputRef}
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Filter rows…"
          className={[
            "w-full rounded border-0 bg-transparent py-0.5 text-xs",
            "placeholder:text-muted-foreground/50",
            "focus-visible:outline-none",
            "text-foreground",
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
            className="absolute right-0 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground transition-colors"
            tabIndex={-1}
            aria-label="Clear filter"
          >
            <X className="h-3 w-3" />
          </button>
        )}
      </div>
      {showCount && (
        <span className="shrink-0 tabular-nums text-[10px] text-muted-foreground">
          {filteredRows.toLocaleString()} of {totalRows.toLocaleString()}
        </span>
      )}
    </div>
  );
}
