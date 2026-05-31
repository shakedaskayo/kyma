import { AlertCircle, CheckCircle2, ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  INTERVAL_PRESETS,
  type ConnectorKindDef,
  type KindFormValues,
} from "./connector-kinds";

/** Inline token-verification state for PAT-auth kinds. */
export interface TokenVerify {
  onVerify: () => void;
  isPending: boolean;
  isError: boolean;
  /** Number of repositories the token can read, when verified. */
  repoCount?: number;
  /** True once a successful verification has returned for the current token. */
  verified: boolean;
}

interface Props {
  kind: ConnectorKindDef;
  values: KindFormValues;
  onChange: (patch: Partial<KindFormValues>) => void;
  advancedOpen: boolean;
  onToggleAdvanced: () => void;
  /** Verification controls for the resource-token field (e.g. the PAT). */
  verify?: TokenVerify;
}

export function ConnectorConfigForm({
  kind,
  values,
  onChange,
  advancedOpen,
  onToggleAdvanced,
  verify,
}: Props) {
  const setField = (key: string, val: string) =>
    onChange({ fields: { ...values.fields, [key]: val } });

  const tokenKey = kind.resourceTokenField;
  const tokenVal = tokenKey ? values.fields[tokenKey] ?? "" : "";

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

      {/* Inline token verification (PAT-auth kinds) */}
      {verify && tokenKey && (
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={verify.onVerify}
            disabled={!tokenVal.trim() || verify.isPending}
            className="inline-flex h-8 items-center gap-1.5 rounded-md border px-3 text-xs font-medium hover:bg-accent disabled:opacity-50"
          >
            {verify.isPending ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
            {verify.isPending ? "Verifying…" : "Verify token"}
          </button>
          {!verify.isPending && verify.verified && (
            <span className="inline-flex items-center gap-1 text-xs text-emerald-600">
              <CheckCircle2 className="h-3.5 w-3.5" />
              {verify.repoCount ?? 0} repositories accessible
            </span>
          )}
          {!verify.isPending && verify.isError && (
            <span className="inline-flex items-center gap-1 text-xs text-destructive">
              <AlertCircle className="h-3.5 w-3.5" />
              Couldn't verify — check the token & its scopes
            </span>
          )}
        </div>
      )}

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
