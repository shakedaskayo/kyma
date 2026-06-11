import { useEffect, useState } from "react";
import { Check, ExternalLink, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { CatalogEntry } from "@/sdk/datasources";
import { useCredentials } from "@/features/credentials/useCredentials";
import { BrandIcon } from "./BrandIcon";
import { useOAuthConnect, type ConnectedAccount } from "./useOAuthConnect";

/**
 * The "Connect <provider>" step for OAuth connectors. Big primary connect
 * button (popup), an advanced disclosure to bring your own client app, and a
 * picker to reuse an existing OAuth credential. Surfaces the minted (or chosen)
 * `credential_id` to the wizard via `onConnected`.
 */
export function ConnectorConnectStep({
  entry,
  credentialId,
  onConnected,
  onClear,
}: {
  entry: CatalogEntry;
  credentialId: string | null;
  onConnected: (a: ConnectedAccount) => void;
  onClear: () => void;
}) {
  const provider = entry.oauth_provider;
  const oauth = useOAuthConnect(provider, entry.type_id);
  const creds = useCredentials();
  const [advanced, setAdvanced] = useState(false);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");

  // Propagate a successful connect up to the wizard.
  useEffect(() => {
    if (oauth.phase === "success" && oauth.account) onConnected(oauth.account);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [oauth.phase, oauth.account]);

  const oauthCreds = (creds.data ?? []).filter((c) => c.kind === "oauth2");

  // ── Connected ───────────────────────────────────────────────────────────────
  if (credentialId) {
    const cred = (creds.data ?? []).find((c) => c.id === credentialId);
    return (
      <div className="flex items-center gap-3 rounded-lg border border-emerald-500/30 bg-emerald-500/5 p-3">
        <div className="flex h-9 w-9 items-center justify-center rounded-md border bg-background">
          <BrandIcon brand={entry.brand} size={20} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-sm font-medium">
            <Check className="h-4 w-4 text-emerald-600" />
            Connected{cred?.label ? ` · ${cred.label}` : ""}
          </div>
          {cred?.preview && (
            <div className="font-mono text-xs text-muted-foreground">{cred.preview}</div>
          )}
        </div>
        <Button variant="ghost" size="sm" onClick={onClear}>
          Use a different account
        </Button>
      </div>
    );
  }

  const connecting = oauth.phase === "starting" || oauth.phase === "awaiting";

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Authorize Kyma to read your {entry.label} data. A secure window opens to {entry.label};
        nothing is stored until you approve.
      </p>

      <Button
        onClick={() =>
          oauth.connect({
            scopes: entry.oauth_scopes,
            clientId: advanced ? clientId.trim() || undefined : undefined,
            clientSecret: advanced ? clientSecret.trim() || undefined : undefined,
          })
        }
        disabled={connecting || !provider}
        className="w-full gap-2"
      >
        {connecting ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <BrandIcon brand={entry.brand} size={16} monochrome />
        )}
        {connecting ? "Waiting for authorization…" : `Connect ${entry.label}`}
      </Button>

      {oauth.popupBlocked && oauth.authorizeUrl && (
        <p className="text-xs text-amber-600">
          Popup blocked.{" "}
          <a className="underline" href={oauth.authorizeUrl} target="_blank" rel="noreferrer">
            Open authorization in a new tab <ExternalLink className="inline h-3 w-3" />
          </a>
        </p>
      )}
      {oauth.phase === "error" && oauth.error && (
        <p className="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {oauth.error}{" "}
          <button type="button" className="underline" onClick={() => oauth.reset()}>
            try again
          </button>
        </p>
      )}

      {/* Advanced — bring your own OAuth app */}
      <div className="rounded-md border">
        <button
          type="button"
          className="flex w-full items-center justify-between px-3 py-2 text-xs text-muted-foreground hover:bg-accent/30"
          onClick={() => setAdvanced((v) => !v)}
        >
          <span>Advanced — use your own OAuth app</span>
          <span aria-hidden>{advanced ? "−" : "+"}</span>
        </button>
        {advanced && (
          <div className="space-y-2 border-t p-3">
            <div className="space-y-1">
              <Label htmlFor="oauth-cid">Client ID</Label>
              <Input
                id="oauth-cid"
                value={clientId}
                onChange={(e) => setClientId(e.target.value)}
                placeholder="your-app-client-id"
              />
            </div>
            <div className="space-y-1">
              <Label htmlFor="oauth-csec">Client secret</Label>
              <Input
                id="oauth-csec"
                type="password"
                autoComplete="off"
                value={clientSecret}
                onChange={(e) => setClientSecret(e.target.value)}
                placeholder="••••••••"
              />
            </div>
            <p className="text-[11px] text-muted-foreground">
              Register the redirect URI{" "}
              <code className="rounded bg-muted px-1">&lt;server&gt;/v1/oauth/{provider}/callback</code>{" "}
              in your provider's app settings.
            </p>
          </div>
        )}
      </div>

      {/* Reuse an existing OAuth credential */}
      {oauthCreds.length > 0 && (
        <div className="space-y-1">
          <Label htmlFor="oauth-existing">…or use an existing credential</Label>
          <select
            id="oauth-existing"
            className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
            defaultValue=""
            onChange={(e) => {
              const id = e.target.value;
              if (id) onConnected({ credentialId: id, label: oauthCreds.find((c) => c.id === id)?.label });
            }}
          >
            <option value="" disabled>
              Select a saved credential…
            </option>
            {oauthCreds.map((c) => (
              <option key={c.id} value={c.id}>
                {c.label} ({c.preview})
              </option>
            ))}
          </select>
        </div>
      )}
    </div>
  );
}
