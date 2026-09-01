import { useMemo } from "react";
import { Input } from "../internal/ui/input";
import { Button } from "../internal/ui/button";
import { Loader2, Play, X } from "lucide-react";
import { parseSearch } from "./discoverGrammar";

type Props = {
  value: string;
  onChange: (v: string) => void;
  onRun: () => void;
  onCancel: () => void;
  running: boolean;
};

/** Persistent query bar — the text IS the query. Validates the grammar
 * client-side for inline feedback; the backend re-parses authoritatively. */
export function QueryBar({ value, onChange, onRun, onCancel, running }: Props) {
  const grammarError = useMemo(() => {
    try {
      parseSearch(value);
      return null;
    } catch (e) {
      return (e as Error).message;
    }
  }, [value]);

  return (
    <div className="pv-flex pv-flex-col pv-gap-1 pv-flex-1 pv-min-w-0">
      <div className="pv-flex pv-gap-2 pv-items-center">
        <Input
          placeholder='Search… e.g. auth service:payments -severity:INFO  (empty = everything)'
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !grammarError) onRun();
          }}
          aria-invalid={grammarError != null}
          className="pv-font-mono pv-text-sm pv-flex-1 pv-min-w-0"
        />
        {running ? (
          <Button variant="destructive" size="sm" onClick={onCancel}>
            <X className="pv-size-4 pv-mr-1" /> Cancel
          </Button>
        ) : (
          <Button size="sm" onClick={onRun} disabled={grammarError != null}>
            <Play className="pv-size-4 pv-mr-1" /> Run
          </Button>
        )}
        {running && <Loader2 className="pv-size-4 pv-animate-spin pv-text-muted-foreground" />}
      </div>
      {grammarError && (
        <div className="pv-text-xs pv-text-destructive pv-px-1" role="alert">
          {grammarError}
        </div>
      )}
    </div>
  );
}
