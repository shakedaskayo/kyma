import { createFileRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useSession } from "@/sdk/session";

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
    <div className="mx-auto max-w-xl p-6">
      <Card>
        <CardHeader><CardTitle>Connect to kyma</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="endpoint">Server URL</Label>
            <Input id="endpoint" value={endpoint} onChange={(e) => setEndpoint(e.target.value)} placeholder="http://localhost:8080" />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="token">Bearer token</Label>
            <Input id="token" type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="paste token from KYMA_AUTH_TOKENS" />
            <p className="text-xs text-muted-foreground">
              Configure with <code className="font-mono">KYMA_AUTH_TOKENS=token:read</code> on the server; leave blank
              and set <code>KYMA_AUTH_DISABLED=1</code> for unauthenticated dev.
            </p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="database">Default database</Label>
            <Input id="database" value={database} onChange={(e) => setDatabase(e.target.value)} placeholder="obs" />
          </div>
          <div className="flex gap-2 pt-2">
            <Button onClick={save}>Save + connect</Button>
            <Button variant="ghost" onClick={() => session.reset()}>Reset</Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
