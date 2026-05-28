import { useEffect, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, Loader2, X } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent } from "@/components/ui/card";
import { useSession } from "@/sdk/session";
import {
  listEngines,
  putEngine,
  testEngine,
  type EngineConfig,
  type EngineKind,
} from "@/sdk/agent-engine";
import { listCredentials } from "@/sdk/credentials";

/**
 * Settings → Agent Engine card.
 *
 * Picks the provider, model, optional credential, and (for Ollama) host.
 * "Test connection" hits the backend probe (1-token live call). "Save"
 * persists. The credential dropdown defaults to "auto-detect from host" —
 * the backend resolves env vars and ~/.claude/.credentials.json in that
 * order when no credential id is attached.
 */
export function EngineSettings() {
  const { endpoint, token } = useSession();
  const qc = useQueryClient();
  const enabled = Boolean(endpoint && token);

  const engines = useQuery({
    queryKey: ["agent-engines", endpoint],
    queryFn: () => listEngines({ endpoint, token }),
    enabled,
  });
  const creds = useQuery({
    queryKey: ["credentials", endpoint],
    queryFn: () => listCredentials({ endpoint, token }),
    enabled,
  });

  const [kind, setKind] = useState<EngineKind>("ollama");
  const [model, setModel] = useState("");
  const [credentialId, setCredentialId] = useState<string | null>(null);
  const [host, setHost] = useState<string | null>(null);
  const [probe, setProbe] = useState<
    { kind: "idle" | "running" } | { kind: "ok"; msg: string } | { kind: "err"; msg: string }
  >({ kind: "idle" });

  // Seed local state from the server's active config when it loads. Re-seed
  // when the underlying data changes (e.g. after Save).
  useEffect(() => {
    if (engines.data) {
      const a = engines.data.active;
      setKind(a.kind);
      setModel(a.model);
      setCredentialId(a.credential_id ?? null);
      setHost(a.host ?? null);
    }
  }, [engines.data]);

  const provider = engines.data?.available.find((e) => e.kind === kind);
  const apiCreds = (creds.data ?? []).filter(
    (c) => c.kind === "api_key" || c.kind === "pat",
  );

  const cfg: EngineConfig = {
    kind,
    model,
    credential_id: credentialId,
    host,
  };

  const save = useMutation({
    mutationFn: () => putEngine({ endpoint, token }, cfg),
    onSuccess: () => {
      toast.success("Engine saved");
      qc.invalidateQueries({ queryKey: ["agent-engines", endpoint] });
    },
    onError: (e: Error) => toast.error(`Save failed: ${e.message}`),
  });

  const runProbe = useMutation({
    mutationFn: () => testEngine({ endpoint, token }, cfg),
    onMutate: () => setProbe({ kind: "running" }),
    onSuccess: (r) =>
      setProbe({ kind: "ok", msg: `${r.kind}/${r.model} reachable` }),
    onError: (e: Error) => setProbe({ kind: "err", msg: e.message }),
  });

  return (
    <Card>
      <CardContent className="space-y-4 p-4">
        <p className="text-xs text-muted-foreground">
          Pick the model Ask Kyma uses. Anthropic / OpenAI keys are
          auto-detected from <code className="font-mono">ANTHROPIC_API_KEY</code>,{" "}
          <code className="font-mono">OPENAI_API_KEY</code>, or{" "}
          <code className="font-mono">~/.claude/.credentials.json</code> when
          no explicit credential is attached.
        </p>

        {engines.isError && (
          <p className="text-sm text-destructive">
            Failed to load engines: {(engines.error as Error).message}
          </p>
        )}

        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="engine-kind">Provider</Label>
            <select
              id="engine-kind"
              value={kind}
              onChange={(e) => {
                const k = e.target.value as EngineKind;
                setKind(k);
                const p = engines.data?.available.find((a) => a.kind === k);
                if (p && !p.models.includes(model)) setModel(p.models[0] ?? "");
              }}
              className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm"
            >
              {engines.data?.available.map((p) => (
                <option key={p.kind} value={p.kind}>
                  {p.label}
                </option>
              ))}
            </select>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="engine-model">Model</Label>
            <select
              id="engine-model"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm"
            >
              {provider?.models.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          </div>
        </div>

        {provider?.needs_key && (
          <div className="space-y-1.5">
            <Label htmlFor="engine-credential">
              Credential{" "}
              <span className="font-normal text-muted-foreground">
                (optional — auto-detect from host if blank)
              </span>
            </Label>
            <select
              id="engine-credential"
              value={credentialId ?? ""}
              onChange={(e) => setCredentialId(e.target.value || null)}
              className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm"
            >
              <option value="">— auto-detect from host —</option>
              {apiCreds.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label} ({c.preview})
                </option>
              ))}
            </select>
          </div>
        )}

        {kind === "ollama" && (
          <div className="space-y-1.5">
            <Label htmlFor="engine-host">Ollama host</Label>
            <Input
              id="engine-host"
              value={host ?? ""}
              onChange={(e) => setHost(e.target.value || null)}
              placeholder="http://localhost:11434"
            />
          </div>
        )}

        <div className="flex items-center gap-2 pt-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => runProbe.mutate()}
            disabled={runProbe.isPending || !model}
          >
            {runProbe.isPending ? (
              <span className="inline-flex items-center gap-1">
                <Loader2 className="h-3.5 w-3.5 animate-spin" /> Testing…
              </span>
            ) : (
              "Test connection"
            )}
          </Button>
          <Button size="sm" onClick={() => save.mutate()} disabled={save.isPending || !model}>
            {save.isPending ? "Saving…" : "Save"}
          </Button>
          {probe.kind === "ok" && (
            <span className="inline-flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-400">
              <Check className="h-3.5 w-3.5" /> {probe.msg}
            </span>
          )}
          {probe.kind === "err" && (
            <span className="inline-flex items-center gap-1 text-xs text-destructive">
              <X className="h-3.5 w-3.5" /> {probe.msg}
            </span>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
