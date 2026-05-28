import { Badge } from "@/components/ui/badge";
import { X } from "lucide-react";
import { serializePills } from "./discoverGrammar";
import type { Pill } from "./types";

type Props = {
  pills: Pill[];
  onRemove: (idx: number) => void;
};

export function FilterPills({ pills, onRemove }: Props) {
  if (pills.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1 items-center">
      {pills.map((p, i) => (
        <Badge
          key={`${i}-${pillKey(p)}`}
          variant="secondary"
          className="font-mono text-xs gap-1 pr-1"
        >
          {serializePills([p])}
          <button
            type="button"
            onClick={() => onRemove(i)}
            aria-label="remove filter pill"
            className="ml-0.5 hover:bg-background/40 rounded p-0.5"
          >
            <X className="size-3" />
          </button>
        </Badge>
      ))}
    </div>
  );
}

function pillKey(p: Pill): string {
  switch (p.kind) {
    case "substring": return `s:${p.value}`;
    case "eq": return `eq:${p.field}:${p.value}`;
    case "neq": return `neq:${p.field}:${p.value}`;
    case "cmp": return `cmp:${p.field}:${p.op}:${p.value}`;
    case "exists": return `ex:${p.field}`;
  }
}
