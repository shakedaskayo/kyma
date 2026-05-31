import { useState, useEffect } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Loader2, Play, X } from "lucide-react";

type Props = {
  value: string;
  onChange: (v: string) => void;
  onSubmit: () => void;
  onCancel: () => void;
  running: boolean;
};

export function SearchBar({ value, onChange, onSubmit, onCancel, running }: Props) {
  const [draft, setDraft] = useState(value);

  // Keep draft in sync if value resets externally (e.g. after submit).
  useEffect(() => {
    setDraft(value);
  }, [value]);

  const fire = () => {
    onChange(draft);
    onSubmit();
  };

  return (
    <div className="flex gap-2 items-center flex-1 min-w-0">
      <Input
        placeholder='Search… e.g. auth service:payments -severity:INFO'
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") fire();
        }}
        className="font-mono text-sm flex-1 min-w-0"
      />
      {running ? (
        <Button variant="destructive" size="sm" onClick={onCancel}>
          <X className="size-4 mr-1" /> Cancel
        </Button>
      ) : (
        <Button size="sm" onClick={fire}>
          <Play className="size-4 mr-1" /> Run
        </Button>
      )}
      {running && <Loader2 className="size-4 animate-spin text-muted-foreground" />}
    </div>
  );
}
