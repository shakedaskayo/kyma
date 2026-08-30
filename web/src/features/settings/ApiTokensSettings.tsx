import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { Copy, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent } from "@/components/ui/card";
import { useSession } from "@/sdk/session";
import {
  createApiToken,
  listApiTokens,
  revokeApiToken,
  type ApiTokenEntry,
} from "@/sdk/tokens";

/**
 * Long-lived API tokens for the CLI / MCP clients / CI. The raw token is
 * shown exactly once after creation — afterwards only metadata is listed.
 */
export function ApiTokensSettings() {
  const session = useSession();
  const [tokens, setTokens] = useState<ApiTokenEntry[]>([]);
  const [name, setName] = useState("");
  const [role, setRole] = useState("read");
  const [creating, setCreating] = useState(false);
  const [minted, setMinted] = useState<string | null>(null);

  const reload = useCallback(() => {
    listApiTokens()
      .then((items) => setTokens(items.filter((t) => !t.revoked)))
      .catch(() => setTokens([]));
  }, []);

  useEffect(() => {
    if (session.token) reload();
  }, [session.token, reload]);

  const handleCreate = async () => {
    setCreating(true);
    try {
      const result = await createApiToken({
        name: name.trim() || undefined,
        role,
      });
      setMinted(result.token);
      setName("");
      reload();
    } catch (err) {
      toast.error((err as Error).message || "Failed to create token");
    } finally {
      setCreating(false);
    }
  };

  const handleRevoke = async (id: string) => {
    try {
      await revokeApiToken(id);
      toast.success("Token revoked");
      reload();
    } catch (err) {
      toast.error((err as Error).message || "Failed to revoke token");
    }
  };

  const copyMinted = async () => {
    if (!minted) return;
    await navigator.clipboard.writeText(minted).catch(() => {});
    toast.success("Token copied");
  };

  if (!session.token) {
    return (
      <Card>
        <CardContent className="p-4 text-sm text-muted-foreground">
          Sign in to manage API tokens.
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      <Card>
        <CardContent className="space-y-4 p-4">
          <div>
            <p className="text-sm font-medium">Create a token</p>
            <p className="mt-0.5 text-xs text-muted-foreground">
              For <code className="font-mono">pensieve connect &lt;url&gt; --token …</code>, MCP
              clients, and CI. The token is shown once — store it safely.
            </p>
          </div>
          <div className="flex flex-wrap items-end gap-3">
            <div className="min-w-40 flex-1 space-y-1.5">
              <Label htmlFor="token-name">Name</Label>
              <Input
                id="token-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="ci-bot"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="token-role">Role</Label>
              <select
                id="token-role"
                value={role}
                onChange={(e) => setRole(e.target.value)}
                className="h-9 rounded-md border border-input bg-transparent px-3 text-sm shadow-sm"
              >
                <option value="read">read</option>
                <option value="write">write</option>
                <option value="admin">admin</option>
              </select>
            </div>
            <Button size="sm" onClick={handleCreate} disabled={creating}>
              {creating ? "Creating…" : "Create token"}
            </Button>
          </div>
          <p className="text-[11px] text-muted-foreground">
            The granted role is capped at your own role.
          </p>

          {minted && (
            <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-3">
              <p className="mb-2 text-xs font-medium">
                Copy this token now — it won't be shown again.
              </p>
              <div className="flex items-center gap-2">
                <code className="min-w-0 flex-1 truncate rounded bg-background/60 px-2 py-1 font-mono text-xs">
                  {minted}
                </code>
                <Button size="sm" variant="outline" onClick={copyMinted}>
                  <Copy className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardContent className="p-4">
          <p className="mb-3 text-sm font-medium">Active tokens</p>
          {tokens.length === 0 ? (
            <p className="text-sm text-muted-foreground">No API tokens yet.</p>
          ) : (
            <ul className="divide-y divide-border">
              {tokens.map((t) => (
                <li key={t.id} className="flex items-center gap-3 py-2.5">
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm">{t.name ?? "unnamed"}</p>
                    <p className="text-xs text-muted-foreground">
                      {t.role}
                      {" · created "}
                      {new Date(t.created_at).toLocaleDateString()}
                      {t.last_used_at
                        ? ` · last used ${new Date(t.last_used_at).toLocaleDateString()}`
                        : " · never used"}
                    </p>
                  </div>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => handleRevoke(t.id)}
                    aria-label={`Revoke ${t.name ?? "token"}`}
                  >
                    <Trash2 className="h-3.5 w-3.5 text-muted-foreground" />
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
