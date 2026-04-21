import { useState, useRef, useEffect } from "react";
import { Search, HelpCircle, X } from "lucide-react";
import { parseSearch, type SearchSchema } from "./parseSearch";

interface SmartSearchBarProps {
  /** The leading table name from the current query (null = disabled) */
  leadingTable: string | null;
  /** Schema for column discovery */
  schema: SearchSchema | null;
  /** Called when user submits a search — receives the generated KQL string */
  onSearch: (kql: string) => void;
  /** Called when the user clears the search bar */
  onClear?: () => void;
}

const GRAMMAR_EXAMPLES = [
  { input: "auth",                                  desc: "Bare word — substring match across all text columns" },
  { input: 'service_name:payments',                  desc: "field:value — exact match" },
  { input: '-severity_text:INFO',                    desc: "-field:value — exclude (!=)" },
  { input: '-debug',                                  desc: "-word — exclude substring" },
  { input: '"auth failure"',                          desc: "Quoted phrase — treated as a single term" },
  { input: 'auth service_name:payments -severity_text:INFO', desc: "Multiple terms — AND'd together" },
];

function HelpPopover({ onClose }: { onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handler(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    }
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="absolute right-0 top-full z-50 mt-1 w-[480px] rounded-lg border border-border bg-popover p-4 shadow-xl text-xs"
      onClick={(e) => e.stopPropagation()}
    >
      <div className="mb-2 font-semibold text-sm text-foreground">Smart Search Grammar</div>
      <p className="mb-3 text-muted-foreground leading-relaxed">
        Type plain text or shorthand field filters. All terms are AND'd together.
        Hit <kbd className="rounded bg-muted px-1 py-0.5 font-mono text-[10px]">Enter</kbd> or click Run to execute.
      </p>
      <table className="w-full border-collapse">
        <thead>
          <tr className="text-[10px] uppercase tracking-wider text-muted-foreground">
            <th className="text-left pb-1 pr-4 font-semibold">Example</th>
            <th className="text-left pb-1 font-semibold">Behaviour</th>
          </tr>
        </thead>
        <tbody>
          {GRAMMAR_EXAMPLES.map(({ input, desc }) => (
            <tr key={input} className="border-t border-muted/40">
              <td className="py-1 pr-4 align-top">
                <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-foreground whitespace-nowrap">
                  {input}
                </code>
              </td>
              <td className="py-1 align-top text-muted-foreground">{desc}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="mt-3 text-[10px] text-muted-foreground italic">
        Tip: the search bar translates to KQL and places it in the editor — you can still edit the raw query afterwards.
      </p>
    </div>
  );
}

export function SmartSearchBar({ leadingTable, schema, onSearch, onClear }: SmartSearchBarProps) {
  const [value, setValue] = useState("");
  const [showHelp, setShowHelp] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const disabled = !leadingTable;

  const handleSubmit = () => {
    if (disabled || !schema) return;
    const kql = parseSearch(value, leadingTable, schema);
    onSearch(kql);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleSubmit();
    }
    if (e.key === "Escape") {
      setValue("");
      onClear?.();
      inputRef.current?.blur();
    }
  };

  const handleClear = () => {
    setValue("");
    onClear?.();
    inputRef.current?.focus();
  };

  return (
    <div className="relative flex items-center gap-2 border-b bg-background px-4 py-2 shadow-sm">
      {/* Search icon */}
      <Search className="h-4 w-4 shrink-0 text-muted-foreground" />

      {/* Main input */}
      <div className="relative flex-1">
        <input
          ref={inputRef}
          type="text"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKeyDown}
          disabled={disabled}
          placeholder={
            disabled
              ? "Write a query first, or use a saved query chip"
              : `Search ${leadingTable} — e.g. auth service_name:payments -severity_text:INFO`
          }
          className={[
            "w-full rounded-md border px-3 py-2 text-sm ring-offset-background transition-colors",
            "placeholder:text-muted-foreground/60",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-0",
            disabled
              ? "cursor-not-allowed border-input/40 bg-muted/30 text-muted-foreground"
              : "border-input bg-background text-foreground hover:border-ring/50",
          ].join(" ")}
          style={{ fontSize: "14px", fontFamily: "inherit" }}
        />
        {/* Clear button */}
        {value && (
          <button
            onClick={handleClear}
            className="absolute right-2 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground transition-colors"
            tabIndex={-1}
            aria-label="Clear search"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      {/* ⌘K shortcut hint */}
      {!disabled && (
        <kbd className="hidden shrink-0 items-center gap-1 rounded border border-muted/60 bg-muted/40 px-1.5 py-0.5 text-[11px] text-muted-foreground sm:flex">
          ⏎ Run
        </kbd>
      )}

      {/* Help button */}
      <div className="relative shrink-0">
        <button
          onClick={() => setShowHelp((v) => !v)}
          className={`rounded p-1 transition-colors ${showHelp ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-accent/40 hover:text-foreground"}`}
          aria-label="Search grammar help"
        >
          <HelpCircle className="h-4 w-4" />
        </button>
        {showHelp && <HelpPopover onClose={() => setShowHelp(false)} />}
      </div>
    </div>
  );
}
