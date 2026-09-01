import { createFileRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useSession } from "@/sdk/session";
import { login, me } from "@/sdk/auth";
import { getSetupStatus } from "@/sdk/setup";
import { fetchAuthConfig, type AuthConfig } from "@/sdk/authConfig";
import { getSupabase } from "@/sdk/supabase";

export const Route = createFileRoute("/login")({
  validateSearch: (s: Record<string, unknown>): { next?: string } => ({
    next: typeof s.next === "string" ? s.next : undefined,
  }),
  component: Login,
});

/** Best default for the server URL: the page's own origin when the SPA is
 *  served by the engine (production), localhost:8080 in dev/Tauri. */
function defaultEndpoint(saved: string): string {
  if (saved) return saved;
  if (
    typeof window !== "undefined" &&
    window.location.origin.startsWith("http") &&
    !window.location.origin.includes("localhost:5173")
  ) {
    return window.location.origin;
  }
  return "http://localhost:8080";
}

const PROVIDER_LABELS: Record<string, string> = {
  google: "Google",
  github: "GitHub",
  gitlab: "GitLab",
  azure: "Microsoft",
  apple: "Apple",
  bitbucket: "Bitbucket",
  discord: "Discord",
  slack: "Slack",
};

function Login() {
  const session = useSession();
  const { next } = useSearch({ from: "/login" });
  const navigate = useNavigate();

  const [endpoint, setEndpoint] = useState(defaultEndpoint(session.endpoint));
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [database, setDatabase] = useState(session.database || "obs");
  const [loading, setLoading] = useState(false);
  const [authConfig, setAuthConfig] = useState<AuthConfig | null>(null);

  const trimSlash = (s: string) => s.replace(/\/$/, "");

  // Discover the server's auth mode (password vs supabase). Re-runs when the
  // endpoint field settles so pointing at a different server flips the form.
  useEffect(() => {
    const ep = trimSlash(endpoint);
    if (!ep) return;
    const t = setTimeout(() => {
      fetchAuthConfig(ep).then(setAuthConfig);
    }, 300);
    return () => clearTimeout(t);
  }, [endpoint]);

  // Fresh instance with no users yet → send the operator to first-run setup.
  // Password mode only: supabase deployments JIT-provision users on first
  // sign-in, so an empty users table is the NORMAL state, not "needs setup".
  useEffect(() => {
    if (!authConfig || authConfig.provider !== "password") return;
    getSetupStatus(trimSlash(endpoint) || "http://localhost:8080")
      .then((s) => {
        if (s.setup_required) navigate({ to: "/setup" });
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authConfig]);

  /** Persist Supabase config into the session store so getSupabase() can
   *  build the client (and survive the OAuth redirect round-trip). */
  const primeSupabaseSession = (cfg: Extract<AuthConfig, { provider: "supabase" }>) => {
    session.set({
      endpoint: trimSlash(endpoint),
      provider: "supabase",
      supabaseUrl: cfg.supabase_url,
      supabaseAnonKey: cfg.supabase_anon_key,
      database: database || "obs",
    });
  };

  const completeSupabaseLogin = async (accessToken: string) => {
    session.set({ token: accessToken });
    // The engine JIT-provisions the user on this first authenticated call.
    const user = await me({ endpoint: trimSlash(endpoint), token: accessToken }).catch(
      () => null,
    );
    if (user) session.set({ user });
    toast.success("Signed in");
    navigate({ to: next ?? "/explore" });
  };

  // OAuth redirect return: supabase-js (detectSessionInUrl) captures the
  // tokens from the URL hash when the client initializes.
  useEffect(() => {
    if (authConfig?.provider !== "supabase" || session.isAuthenticated()) return;
    primeSupabaseSession(authConfig);
    (async () => {
      const sb = await getSupabase();
      if (!sb) return;
      const { data } = await sb.auth.getSession();
      if (data.session?.access_token) {
        await completeSupabaseLogin(data.session.access_token);
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authConfig]);

  const handleSignIn = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!endpoint || !username || !password) {
      toast.error(
        authConfig?.provider === "supabase"
          ? "Server URL, email, and password are required."
          : "Server URL, username, and password are required.",
      );
      return;
    }
    setLoading(true);
    try {
      if (authConfig?.provider === "supabase") {
        primeSupabaseSession(authConfig);
        const sb = await getSupabase();
        if (!sb) throw new Error("Supabase client unavailable");
        const { data, error } = await sb.auth.signInWithPassword({
          email: username,
          password,
        });
        if (error || !data.session) {
          throw new Error(error?.message ?? "sign in failed");
        }
        await completeSupabaseLogin(data.session.access_token);
      } else {
        const result = await login({ endpoint: trimSlash(endpoint), username, password });
        session.set({
          endpoint: trimSlash(endpoint),
          provider: "password",
          token: result.access_token,
          refreshToken: result.refresh_token,
          accessExpiresAt: result.access_expires_at,
          user: result.user,
          database: database || "obs",
        });
        toast.success("Signed in");
        navigate({ to: next ?? "/explore" });
      }
    } catch (err) {
      const msg = (err as Error).message;
      if (
        msg.toLowerCase().includes("unauthorized") ||
        msg.toLowerCase().includes("invalid login credentials") ||
        msg.includes("401") ||
        msg.includes("403")
      ) {
        toast.error("Invalid credentials");
      } else {
        toast.error(msg || "Sign in failed");
      }
    } finally {
      setLoading(false);
    }
  };

  const handleOAuth = async (provider: string) => {
    if (authConfig?.provider !== "supabase") return;
    primeSupabaseSession(authConfig);
    const sb = await getSupabase();
    if (!sb) {
      toast.error("Supabase client unavailable");
      return;
    }
    const { error } = await sb.auth.signInWithOAuth({
      // Provider ids come from the server's PENSIEVE_SUPABASE_PROVIDERS config.
      provider: provider as Parameters<typeof sb.auth.signInWithOAuth>[0]["provider"],
      options: { redirectTo: `${window.location.origin}/login` },
    });
    if (error) toast.error(error.message);
    // On success the browser navigates away to the provider.
  };

  const supabaseMode = authConfig?.provider === "supabase";
  const oauthProviders = supabaseMode
    ? (authConfig as Extract<AuthConfig, { provider: "supabase" }>).oauth_providers
    : [];

  return (
    <div className="app-bg flex min-h-screen items-center justify-center p-4">
      <div className="grid w-full max-w-4xl items-center gap-10 lg:grid-cols-2 lg:gap-16">
        {/* Hero (large screens) */}
        <div className="hidden flex-col justify-center lg:flex">
          <div className="flex items-center gap-2.5">
            <img src="/icons/pensieve-mark.svg" alt="pensieve" className="h-10 w-10" />
            <span className="text-2xl font-semibold tracking-tight">pensieve</span>
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
              <img src="/icons/pensieve-mark.svg" alt="pensieve" className="h-8 w-8" />
              <span className="text-lg font-semibold tracking-tight">pensieve</span>
            </div>
            <h2 className="mb-4 text-lg font-semibold tracking-tight">Sign in</h2>

            {oauthProviders.length > 0 && (
              <div className="mb-4 space-y-2">
                {oauthProviders.map((p) => (
                  <Button
                    key={p}
                    type="button"
                    variant="outline"
                    className="w-full"
                    onClick={() => handleOAuth(p)}
                  >
                    Continue with {PROVIDER_LABELS[p] ?? p}
                  </Button>
                ))}
                <div className="flex items-center gap-3 pt-1">
                  <div className="h-px flex-1 bg-border" />
                  <span className="text-[11px] uppercase tracking-wider text-muted-foreground">
                    or
                  </span>
                  <div className="h-px flex-1 bg-border" />
                </div>
              </div>
            )}

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
                <Label htmlFor="username">{supabaseMode ? "Email" : "Username"}</Label>
                <Input
                  id="username"
                  type={supabaseMode ? "email" : "text"}
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  placeholder={supabaseMode ? "you@company.com" : "admin"}
                  autoComplete={supabaseMode ? "email" : "username"}
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
