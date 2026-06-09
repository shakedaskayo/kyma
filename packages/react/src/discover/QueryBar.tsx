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
    <div className="ky-flex ky-flex-col ky-gap-1 ky-flex-1 ky-min-w-0">
      <div className="ky-flex ky-gap-2 ky-items-center">
        <Input
          placeholder='Search… e.g. auth service:payments -severity:INFO  (empty = everything)'
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !grammarError) onRun();
          }}
          aria-invalid={grammarError != null}
          className="ky-font-mono ky-text-sm ky-flex-1 ky-min-w-0"
        />
        {running ? (
          <Button variant="destructive" size="sm" onClick={onCancel}>
            <X className="ky-size-4 ky-mr-1" /> Cancel
          </Button>
        ) : (
          <Button size="sm" onClick={onRun} disabled={grammarError != null}>
            <Play className="ky-size-4 ky-mr-1" /> Run
          </Button>
        )}
        {running && <Loader2 className="ky-size-4 ky-animate-spin ky-text-muted-foreground" />}
      </div>
      {grammarError && (
        <div className="ky-text-xs ky-text-destructive ky-px-1" role="alert">
          {grammarError}
        </div>
      )}
    </div>
  );
}
