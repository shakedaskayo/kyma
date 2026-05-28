import { createFileRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { ChevronDown } from "lucide-react";
import { useSession } from "@/sdk/session";
import { logout } from "@/sdk/auth";
import { EngineSettings } from "@/features/settings/EngineSettings";

export const Route = createFileRoute("/settings")({
  validateSearch: (s: Record<string, unknown>) => ({ next: typeof s.next === "string" ? s.next : "/explore" }),
  component: Settings,
});

function Settings() {
  const session = useSession();
  const { next } = useSearch({ from: "/settings" });
  const navigate = useNavigate();
  const [endpoint, setEndpoint] = useState(session.endpoint || "http://localhost:8080");
  const [token, setToken]       = useState(session.token);
  const [database, setDatabase] = useState(session.database || "obs");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);

  const handleLogout = async () => {
    setLoggingOut(true);
    try {
      if (session.token) {
        await logout({ endpoint: session.endpoint, token: session.token }).catch(() => {
          // Best-effort: even if the server call fails, clear local session.
        });
      }
    } finally {
      session.reset();
      setLoggingOut(false);
      navigate({ to: "/login", search: { next: undefined } });
    }
  };

  const save = async () => {
    try {
      const ping = await fetch(`${endpoint.replace(/\/$/, "")}/health`);
      if (!ping.ok) throw new Error(`health ${ping.status}`);
    } catch (e) {
      toast.error(`Can't reach ${endpoint}: ${(e as Error).message}`);
      return;
    }
    session.set({ endpoint: endpoint.replace(/\/$/, ""), token, database });
    toast.success("Saved. Connected to kyma.");
    navigate({ to: next as string });
  };

  return (
    <div className="mx-auto max-w-xl p-6 space-y-4">

      {/* Account section — only shown when logged in */}
      {session.user && (
        <Card>
          <CardHeader><CardTitle>Account</CardTitle></CardHeader>
          <CardContent className="space-y-4">
            <div className="flex items-center justify-between">
              <div className="text-sm">
                Signed in as{" "}
                <span className="font-semibold">{session.user.username}</span>{" "}
                <span className="text-muted-foreground">({session.user.role})</span>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={handleLogout}
                disabled={loggingOut}
              >
                {loggingOut ? "Logging out…" : "Log out"}
              </Button>
            </div>
            <div className="text-xs text-muted-foreground">
              Server: <span className="font-mono">{session.endpoint || "—"}</span>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Agent engine — provider + model + credential */}
      <EngineSettings />

      {/* Connection / advanced settings */}
      <Card>
        <CardHeader><CardTitle>Connection</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="endpoint">Server URL</Label>
            <Input id="endpoint" value={endpoint} onChange={(e) => setEndpoint(e.target.value)} placeholder="http://localhost:8080" />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="database">Default database</Label>
            <Input id="database" value={database} onChange={(e) => setDatabase(e.target.value)} placeholder="obs" />
          </div>

          {/* API token (advanced) — for connecting with a static token instead of login */}
          <Collapsible open={advancedOpen} onOpenChange={setAdvancedOpen}>
            <CollapsibleTrigger asChild>
              <button
                type="button"
                className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
              >
                <ChevronDown
                  className={`h-3.5 w-3.5 transition-transform ${advancedOpen ? "rotate-180" : ""}`}
                />
                API token (advanced)
              </button>
            </CollapsibleTrigger>
            <CollapsibleContent className="space-y-1.5 pt-2">
              <Label htmlFor="token">
                Bearer token{" "}
                <span className="font-normal text-muted-foreground">(optional)</span>
              </Label>
              <Input
                id="token"
                type="password"
                value={token}
                onChange={(e) => setToken(e.target.value)}
                placeholder="paste token from KYMA_AUTH_TOKENS"
              />
              <p className="text-xs text-muted-foreground">
                Configure with <code className="font-mono">KYMA_AUTH_TOKENS=token:read</code> on the server; leave blank
                and set <code>KYMA_AUTH_DISABLED=1</code> for unauthenticated dev.
              </p>
            </CollapsibleContent>
          </Collapsible>

          <div className="flex gap-2 pt-2">
            <Button onClick={save}>Save + connect</Button>
            <Button variant="ghost" onClick={() => session.reset()}>Reset</Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
