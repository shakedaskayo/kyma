import { ChevronDown, ChevronRight } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  INTERVAL_PRESETS,
  type ConnectorKindDef,
  type KindFormValues,
} from "./connector-kinds";

interface Props {
  kind: ConnectorKindDef;
  values: KindFormValues;
  onChange: (patch: Partial<KindFormValues>) => void;
  advancedOpen: boolean;
  onToggleAdvanced: () => void;
}

export function ConnectorConfigForm({
  kind,
  values,
  onChange,
  advancedOpen,
  onToggleAdvanced,
}: Props) {
  const setField = (key: string, val: string) =>
    onChange({ fields: { ...values.fields, [key]: val } });

  return (
    <div className="space-y-4">
      {/* Name */}
      <div className="space-y-1">
        <Label htmlFor="conn-name">Name *</Label>
        <Input
          id="conn-name"
          value={values.name}
          onChange={(e) => onChange({ name: e.target.value })}
          placeholder={`My ${kind.label} connector`}
          autoFocus
        />
      </div>

      {/* Schema-driven fields (PAT, etc.) */}
      {kind.fields.map((f) => (
        <div key={f.key} className="space-y-1">
          <Label htmlFor={`conn-field-${f.key}`}>
            {f.label}
            {f.required && " *"}
          </Label>
          <Input
            id={`conn-field-${f.key}`}
            type={f.type === "secret" ? "password" : "text"}
            autoComplete={f.type === "secret" ? "off" : undefined}
            value={values.fields[f.key] ?? ""}
            onChange={(e) => setField(f.key, e.target.value)}
            placeholder={f.placeholder}
          />
          {f.help && <p className="text-xs text-muted-foreground">{f.help}</p>}
        </div>
      ))}

      {/* Sync interval */}
      <div className="space-y-1">
        <Label htmlFor="conn-interval">Sync interval</Label>
        <select
          id="conn-interval"
          className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          value={values.scheduleMs}
          onChange={(e) => onChange({ scheduleMs: Number(e.target.value) })}
        >
          {INTERVAL_PRESETS.map((p) => (
            <option key={p.ms} value={p.ms}>
              {p.label}
            </option>
          ))}
        </select>
      </div>

      {/* Advanced (target db/table) */}
      <div>
        <button
          type="button"
          className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
          onClick={onToggleAdvanced}
        >
          {advancedOpen ? (
            <ChevronDown className="h-3.5 w-3.5" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5" />
          )}
          Advanced
        </button>
        {advancedOpen && (
          <div className="mt-2 grid grid-cols-2 gap-2 rounded-md border bg-muted/20 p-3">
            <div className="space-y-1">
              <Label htmlFor="conn-db">Target database</Label>
              <Input
                id="conn-db"
                value={values.targetDatabase}
                onChange={(e) => onChange({ targetDatabase: e.target.value })}
                placeholder="(session default)"
              />
            </div>
            <div className="space-y-1">
              <Label htmlFor="conn-table">Target table</Label>
              <Input
                id="conn-table"
                value={values.targetTable}
                onChange={(e) => onChange({ targetTable: e.target.value })}
                placeholder={kind.defaultTargetTable}
              />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
