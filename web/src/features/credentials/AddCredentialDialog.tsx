import { useState } from "react";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  credentialKindLabel,
  type CredentialKind,
  type CredentialValue,
} from "@/sdk/credentials";
import { useCreateCredential } from "./useCredentials";
import { CredentialIcon } from "./CredentialIcon";

// Each kind's form is small + bespoke — a handful of typed inputs that map 1:1
// to the tagged union variant. Keeping them inline (vs. a generic schema-form)
// keeps the wizard sharp and the variant shapes obvious.

const KINDS: CredentialKind[] = ["pat", "basic", "oauth2", "url", "aws_creds", "api_key", "github_app", "service_principal"];

const KIND_DESCRIPTION: Record<CredentialKind, string> = {
  pat: "A single bearer token (GitHub/GitLab PAT, project token, …).",
  basic: "Username + password (Bitbucket app password, basic-auth APIs).",
  oauth2: "Access token, optional refresh + expiry. Long-lived OAuth flows.",
  url: "A connection URL that carries its own credentials (Postgres, MySQL, …).",
  aws_creds: "AWS-style static access keys, with optional session token.",
  api_key: "A generic API key, optionally sent under a custom header.",
  github_app: "GitHub App: app id + installation id + PEM private key.",
  service_principal: "Entra ID (Azure AD) service principal — Microsoft Fabric, Azure SQL.",
};

export function AddCredentialDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const create = useCreateCredential();
  const [kind, setKind] = useState<CredentialKind>("pat");
  const [label, setLabel] = useState("");
  const [fields, setFields] = useState<Record<string, string>>({});

  const reset = () => {
    setKind("pat");
    setLabel("");
    setFields({});
    create.reset();
  };
  const close = () => {
    reset();
    onClose();
  };

  const set = (k: string, v: string) => setFields((f) => ({ ...f, [k]: v }));

  const valid = label.trim().length > 0 && validForKind(kind, fields);

  const submit = () => {
    const value = toValue(kind, fields);
    if (!value) return;
    create.mutate(
      { label: label.trim(), value },
      { onSuccess: () => close() },
    );
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Add credential</DialogTitle>
        </DialogHeader>

        <div className="space-y-4 py-1">
          {/* Kind picker — small chip row */}
          <div className="space-y-1">
            <Label>Type</Label>
            <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-3">
              {KINDS.map((k) => (
                <button
                  key={k}
                  type="button"
                  onClick={() => { setKind(k); setFields({}); }}
                  className={`flex items-center gap-1.5 rounded-md border px-2 py-1.5 text-left text-xs transition ${
                    kind === k ? "border-primary bg-primary/5" : "hover:border-primary/50 hover:bg-accent/30"
                  }`}
                >
                  <CredentialIcon kind={k} size={14} />
                  <span className="truncate font-medium">{credentialKindLabel(k)}</span>
                </button>
              ))}
            </div>
            <p className="text-xs text-muted-foreground">{KIND_DESCRIPTION[kind]}</p>
          </div>

          {/* Label */}
          <div className="space-y-1">
            <Label htmlFor="cred-label">Label *</Label>
            <Input
              id="cred-label"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="e.g. github-readonly"
              autoFocus
            />
          </div>

          {/* Kind-specific fields */}
          <KindFields kind={kind} fields={fields} set={set} />

          {create.isError && (
            <p className="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
              {(create.error as Error)?.message ?? "Save failed."}
            </p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={close}>Cancel</Button>
          <Button onClick={submit} disabled={!valid || create.isPending}>
            {create.isPending ? "Saving…" : "Save credential"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ── Per-kind field forms ─────────────────────────────────────────────────────

function KindFields({
  kind,
  fields,
  set,
}: {
  kind: CredentialKind;
  fields: Record<string, string>;
  set: (k: string, v: string) => void;
}) {
  switch (kind) {
    case "pat":
      return (
        <Row label="Token" name="token" type="password" placeholder="ghp_…" value={fields.token} onChange={set} required />
      );
    case "basic":
      return (
        <>
          <Row label="Username" name="username" placeholder="me@example.com" value={fields.username} onChange={set} required />
          <Row label="Password" name="password" type="password" placeholder="••••••••" value={fields.password} onChange={set} required />
        </>
      );
    case "oauth2":
      return (
        <>
          <Row label="Access token" name="access_token" type="password" placeholder="ya29.…" value={fields.access_token} onChange={set} required />
          <Row label="Refresh token (optional)" name="refresh_token" type="password" placeholder="1//…" value={fields.refresh_token} onChange={set} />
          <Row label="Expires at (ISO, optional)" name="expires_at" placeholder="2026-12-31T00:00:00Z" value={fields.expires_at} onChange={set} />
        </>
      );
    case "url":
      return (
        <Row label="Connection string" name="connection_string" type="password" placeholder="postgres://user:pass@host:5432/db" value={fields.connection_string} onChange={set} required />
      );
    case "aws_creds":
      return (
        <>
          <Row label="Access key ID" name="access_key_id" placeholder="AKIA…" value={fields.access_key_id} onChange={set} required />
          <Row label="Secret access key" name="secret_access_key" type="password" placeholder="••••••••" value={fields.secret_access_key} onChange={set} required />
          <Row label="Session token (optional)" name="session_token" type="password" value={fields.session_token} onChange={set} />
        </>
      );
    case "api_key":
      return (
        <>
          <Row label="Value" name="value" type="password" placeholder="key-…" value={fields.value} onChange={set} required />
          <Row label="Header name (optional, default Authorization)" name="header_name" placeholder="X-API-Key" value={fields.header_name} onChange={set} />
        </>
      );
    case "github_app":
      return (
        <>
          <Row label="App ID" name="app_id" placeholder="12345" value={fields.app_id} onChange={set} required />
          <Row label="Installation ID" name="installation_id" placeholder="67890" value={fields.installation_id} onChange={set} required />
          <div className="space-y-1">
            <Label htmlFor="cred-private_key_pem">Private key (PEM) *</Label>
            <textarea
              id="cred-private_key_pem"
              value={fields.private_key_pem ?? ""}
              onChange={(e) => set("private_key_pem", e.target.value)}
              placeholder="-----BEGIN RSA PRIVATE KEY-----…"
              className="h-32 w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            />
          </div>
        </>
      );
    case "service_principal":
      return (
        <>
          <Row label="Directory (tenant) ID" name="tenant_id" placeholder="00000000-0000-0000-0000-000000000000" value={fields.tenant_id} onChange={set} required />
          <Row label="Application (client) ID" name="client_id" placeholder="00000000-0000-0000-0000-000000000000" value={fields.client_id} onChange={set} required />
          <Row label="Client secret" name="client_secret" type="password" placeholder="••••••••" value={fields.client_secret} onChange={set} required />
        </>
      );
  }
}

function Row({
  label, name, value, onChange, type = "text", placeholder, required = false,
}: {
  label: string; name: string; value?: string; onChange: (k: string, v: string) => void;
  type?: "text" | "password"; placeholder?: string; required?: boolean;
}) {
  return (
    <div className="space-y-1">
      <Label htmlFor={`cred-${name}`}>{label}{required && " *"}</Label>
      <Input
        id={`cred-${name}`}
        type={type}
        autoComplete={type === "password" ? "off" : undefined}
        value={value ?? ""}
        onChange={(e) => onChange(name, e.target.value)}
        placeholder={placeholder}
      />
    </div>
  );
}

// ── Validation + assembly ─────────────────────────────────────────────────────

function required(fields: Record<string, string>, keys: string[]): boolean {
  return keys.every((k) => (fields[k] ?? "").trim().length > 0);
}

function validForKind(kind: CredentialKind, fields: Record<string, string>): boolean {
  switch (kind) {
    case "pat": return required(fields, ["token"]);
    case "basic": return required(fields, ["username", "password"]);
    case "oauth2": return required(fields, ["access_token"]);
    case "url": return required(fields, ["connection_string"]);
    case "aws_creds": return required(fields, ["access_key_id", "secret_access_key"]);
    case "api_key": return required(fields, ["value"]);
    case "github_app": return required(fields, ["app_id", "installation_id", "private_key_pem"]);
    case "service_principal": return required(fields, ["tenant_id", "client_id", "client_secret"]);
  }
}

function toValue(kind: CredentialKind, f: Record<string, string>): CredentialValue | null {
  const g = (k: string) => (f[k] ?? "").trim();
  const gOpt = (k: string) => {
    const v = g(k);
    return v.length ? v : undefined;
  };
  switch (kind) {
    case "pat": return { kind, token: g("token") };
    case "basic": return { kind, username: g("username"), password: g("password") };
    case "oauth2":
      return {
        kind,
        access_token: g("access_token"),
        refresh_token: gOpt("refresh_token"),
        expires_at: gOpt("expires_at"),
      };
    case "url": return { kind, connection_string: g("connection_string") };
    case "aws_creds":
      return {
        kind,
        access_key_id: g("access_key_id"),
        secret_access_key: g("secret_access_key"),
        session_token: gOpt("session_token"),
      };
    case "api_key":
      return { kind, value: g("value"), header_name: gOpt("header_name") };
    case "github_app":
      return {
        kind,
        app_id: g("app_id"),
        installation_id: g("installation_id"),
        private_key_pem: g("private_key_pem"),
      };
    case "service_principal":
      return {
        kind,
        tenant_id: g("tenant_id"),
        client_id: g("client_id"),
        client_secret: g("client_secret"),
      };
  }
}
