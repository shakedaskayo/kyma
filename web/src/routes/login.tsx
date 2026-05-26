import { createFileRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useSession } from "@/sdk/session";
import { login } from "@/sdk/auth";

export const Route = createFileRoute("/login")({
  validateSearch: (s: Record<string, unknown>): { next?: string } => ({
    next: typeof s.next === "string" ? s.next : undefined,
  }),
  component: Login,
});

function Login() {
  const session = useSession();
  const { next } = useSearch({ from: "/login" });
  const navigate = useNavigate();

  const [endpoint, setEndpoint] = useState(session.endpoint || "http://localhost:8080");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [database, setDatabase] = useState(session.database || "obs");
  const [loading, setLoading] = useState(false);

  const trimSlash = (s: string) => s.replace(/\/$/, "");

  const handleSignIn = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!endpoint || !username || !password) {
      toast.error("Server URL, username, and password are required.");
      return;
    }
    setLoading(true);
    try {
      const result = await login({ endpoint: trimSlash(endpoint), username, password });
      session.set({
        endpoint: trimSlash(endpoint),
        token: result.token,
        user: result.user,
        database: database || "obs",
      });
      toast.success("Signed in");
      navigate({ to: next ?? "/explore" });
    } catch (err) {
      const msg = (err as Error).message;
      // Show a friendly message for auth failures.
      if (msg.toLowerCase().includes("unauthorized") || msg.includes("401") || msg.includes("403")) {
        toast.error("Invalid credentials");
      } else {
        toast.error(msg || "Sign in failed");
      }
    } finally {
      setLoading(false);
    }
  };

  const handleConnectWithoutSignIn = async () => {
    if (!endpoint) {
      toast.error("Server URL is required.");
      return;
    }
    setLoading(true);
    try {
      const ping = await fetch(`${trimSlash(endpoint)}/health`);
      if (!ping.ok) throw new Error(`health check returned ${ping.status}`);
      session.set({
        endpoint: trimSlash(endpoint),
        token: "",
        user: null,
        database: database || "obs",
      });
      toast.success("Connected");
      navigate({ to: next ?? "/explore" });
    } catch (err) {
      toast.error(`Can't reach ${endpoint}: ${(err as Error).message}`);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-4">
      <div className="w-full max-w-sm">
        <Card>
          <CardHeader>
            <CardTitle>Sign in to kyma</CardTitle>
          </CardHeader>
          <CardContent>
            <form onSubmit={handleSignIn} className="space-y-4">
              <div className="space-y-1.5">
                <Label htmlFor="endpoint">Server URL</Label>
                <Input
                  id="endpoint"
                  value={endpoint}
                  onChange={(e) => setEndpoint(e.target.value)}
                  placeholder="http://localhost:8080"
                  autoComplete="url"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="username">Username</Label>
                <Input
                  id="username"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  placeholder="admin"
                  autoComplete="username"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="password">Password</Label>
                <Input
                  id="password"
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder=""
                  autoComplete="current-password"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="database">Default database</Label>
                <Input
                  id="database"
                  value={database}
                  onChange={(e) => setDatabase(e.target.value)}
                  placeholder="obs"
                />
              </div>
              <div className="pt-2 space-y-2">
                <Button type="submit" className="w-full" disabled={loading}>
                  {loading ? "Signing in…" : "Sign in"}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  className="w-full text-xs text-muted-foreground"
                  onClick={handleConnectWithoutSignIn}
                  disabled={loading}
                >
                  Connect without sign-in (unauthenticated dev server)
                </Button>
              </div>
            </form>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
