import { cn } from "@/lib/utils";
import { CONNECTOR_KINDS, type ConnectorKindDef } from "./connector-kinds";

export function KindPicker({
  selected,
  onSelect,
}: {
  selected: string | null;
  onSelect: (kind: ConnectorKindDef) => void;
}) {
  return (
    <div className="grid grid-cols-2 gap-3">
      {CONNECTOR_KINDS.map((k) => {
        const Icon = k.icon;
        return (
          <button
            key={k.id}
            type="button"
            onClick={() => onSelect(k)}
            className={cn(
              "flex items-center gap-3 rounded-lg border p-4 text-left transition hover:border-primary/50 hover:bg-accent/30",
              selected === k.id && "border-primary bg-primary/5",
            )}
          >
            <div className="flex h-9 w-9 items-center justify-center rounded-md bg-primary/10 text-primary/80">
              <Icon className="h-4 w-4" />
            </div>
            <div>
              <div className="text-sm font-medium">{k.label}</div>
              <div className="text-xs text-muted-foreground capitalize">{k.auth.mode} auth</div>
            </div>
          </button>
        );
      })}
    </div>
  );
}
