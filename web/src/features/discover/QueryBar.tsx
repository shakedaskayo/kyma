import { useMemo } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
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
    <div className="flex flex-col gap-1 flex-1 min-w-0">
      <div className="flex gap-2 items-center">
        <Input
          placeholder='Search… e.g. auth service:payments -severity:INFO  (empty = everything)'
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !grammarError) onRun();
          }}
          aria-invalid={grammarError != null}
          className="font-mono text-sm flex-1 min-w-0"
        />
        {running ? (
          <Button variant="destructive" size="sm" onClick={onCancel}>
            <X className="size-4 mr-1" /> Cancel
          </Button>
        ) : (
          <Button size="sm" onClick={onRun} disabled={grammarError != null}>
            <Play className="size-4 mr-1" /> Run
          </Button>
        )}
        {running && <Loader2 className="size-4 animate-spin text-muted-foreground" />}
      </div>
      {grammarError && (
        <div className="text-xs text-destructive px-1" role="alert">
          {grammarError}
        </div>
      )}
    </div>
  );
}
