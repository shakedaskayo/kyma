import { createFileRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useSession } from "@/sdk/session";
import { login } from "@/sdk/auth";
import { getSetupStatus } from "@/sdk/setup";

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

  // Fresh instance with no users yet → send the operator to first-run setup.
  useEffect(() => {
    getSetupStatus(session.endpoint || "http://localhost:8080")
      .then((s) => {
        if (s.setup_required) navigate({ to: "/setup" });
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
        token: result.access_token,
        refreshToken: result.refresh_token,
        accessExpiresAt: result.access_expires_at,
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

  return (
    <div className="app-bg flex min-h-screen items-center justify-center p-4">
      <div className="grid w-full max-w-4xl items-center gap-10 lg:grid-cols-2 lg:gap-16">
        {/* Hero (large screens) */}
        <div className="hidden flex-col justify-center lg:flex">
          <div className="flex items-center gap-2.5">
            <img src="/icons/kyma-mark.svg" alt="kyma" className="h-10 w-10" />
            <span className="text-2xl font-semibold tracking-tight">kyma</span>
          </div>
          <h1 className="mt-7 text-3xl font-semibold tracking-tight text-foreground">
            Your data, as a living knowledge graph.
          </h1>
          <p className="mt-3 max-w-md text-sm leading-relaxed text-muted-foreground">
            Query, explore, and let agents reason over a unified memory graph spanning
            every connected source.
          </p>
        </div>

        {/* Sign-in card */}
        <div className="mx-auto w-full max-w-sm">
          <div className="glass rounded-xl p-6 shadow-elev-3">
            <div className="mb-5 flex items-center gap-2 lg:hidden">
              <img src="/icons/kyma-mark.svg" alt="kyma" className="h-8 w-8" />
              <span className="text-lg font-semibold tracking-tight">kyma</span>
            </div>
            <h2 className="mb-4 text-lg font-semibold tracking-tight">Sign in</h2>
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
              <div className="pt-2">
                <Button type="submit" className="w-full" disabled={loading}>
                  {loading ? "Signing in…" : "Sign in"}
                </Button>
              </div>
            </form>
          </div>
        </div>
      </div>
    </div>
  );
}
