import { KeyRound } from "lucide-react";
import { Label } from "@/components/ui/label";
import type { CatalogEntry } from "@/sdk/datasources";
import { credentialKindLabel } from "@/sdk/credentials";
import { useCredentials } from "@/features/credentials/useCredentials";
import { CredentialIcon } from "@/features/credentials/CredentialIcon";
import { acceptedCredentialKinds, requiresStoredCredential } from "./datasource-kinds";

/**
 * Stored-credential picker for non-OAuth credential-backed data sources
 * (e.g. msfabric's Entra service principal). Lists credentials filtered to
 * the entry's `accepted_credential_kinds`; the selection flows into the
 * data source config as `credential_id` via `buildCreateBody`.
 */
export function DataSourceCredentialPicker({
  entry,
  credentialId,
  onChange,
}: {
  entry: CatalogEntry;
  credentialId: string | null;
  onChange: (id: string | null) => void;
}) {
  const creds = useCredentials();
  const kinds = acceptedCredentialKinds(entry);
  const required = requiresStoredCredential(entry);
  const matching = (creds.data ?? []).filter((c) => kinds.includes(c.kind));
  const kindLabels = kinds.map(credentialKindLabel).join(" / ");

  return (
    <div className="space-y-1">
      <Label htmlFor="ds-credential">
        Credential{required && " *"}
        <span className="ml-1 font-normal text-muted-foreground">({kindLabels})</span>
      </Label>
      {matching.length > 0 ? (
        <select
          id="ds-credential"
          className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          value={credentialId ?? ""}
          onChange={(e) => onChange(e.target.value || null)}
        >
          <option value="">{required ? "Select a credential…" : "None (use inline secrets)"}</option>
          {matching.map((c) => (
            <option key={c.id} value={c.id}>
              {c.label} · {c.preview}
            </option>
          ))}
        </select>
      ) : (
        <div className="flex items-start gap-2 rounded-md border border-dashed bg-muted/30 p-3 text-xs text-muted-foreground">
          <KeyRound className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>
            No {kindLabels} credential stored yet. Add one under{" "}
            <span className="font-medium text-foreground">Settings → Credentials</span>, then
            return here — the data source resolves it by reference, so a single rotation
            propagates.
          </span>
        </div>
      )}
      {credentialId && (
        <p className="flex items-center gap-1 text-xs text-muted-foreground">
          <CredentialIcon kind={matching.find((c) => c.id === credentialId)?.kind ?? ""} size={12} />
          Stored encrypted; the data source reads it at run time.
        </p>
      )}
    </div>
  );
}
