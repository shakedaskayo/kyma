import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import {
  AlertCircle,
  ArrowRight,
  Check,
  ChevronDown,
  Copy,
  Database,
  KeyRound,
  Loader2,
  RefreshCw,
  Sparkles,
  Terminal,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useSession } from "@/sdk/session";
import {
  listEngines,
  putEngine,
  type EngineConfig,
  type EngineKind,
  type EngineList,
} from "@/sdk/agent-engine";
import { createCredential } from "@/sdk/credentials";
import {
  getSetupProbe,
  getSetupStatus,
  ingestSample,
  signup,
  type SetupProbe,
} from "@/sdk/setup";

export const Route = createFileRoute("/setup")({
  component: Setup,
});

// ── Kyma brand tokens (from the kyma docs site) ────────────────────────────
const GREEN = "#7ed957"; // phosphor accent
const inputCls =
  "h-11 rounded-lg border-[#dfdfd6]/12 bg-[#111317] text-[#e8e6e0] placeholder:text-[#dfdfd6]/30 " +
  "focus-visible:ring-1 focus-visible:ring-[#7ed957]/60 focus-visible:border-[#7ed957]/50";

type StepId = "agent" | "account" | "engine" | "data" | "done";
const STEPS: { id: StepId; label: string }[] = [
  { id: "agent", label: "Coding agent" },
  { id: "account", label: "Account" },
  { id: "engine", label: "AI engine" },
  { id: "data", label: "Data" },
  { id: "done", label: "Ready" },
];

const trimSlash = (s: string) => s.replace(/\/$/, "");

function Setup() {
  const navigate = useNavigate();
  const session = useSession();

  // Land on the app after setup. `to` is string-typed to use the loose
  // navigation signature (same pattern as login.tsx).
  const enterApp = () => {
    const to: string = "/discover";
    void navigate({ to });
  };

  const [endpoint, setEndpoint] = useState(session.endpoint || "http://localhost:8080");
  const [step, setStep] = useState<StepId>("agent");
  const stepIndex = STEPS.findIndex((s) => s.id === step);

  const [checking, setChecking] = useState(true);
  useEffect(() => {
    let alive = true;
    getSetupStatus(endpoint)
      .then((s) => {
        if (!alive) return;
        if (!s.setup_required) navigate({ to: "/login" });
        else setChecking(false);
      })
      .catch(() => alive && setChecking(false));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);

  async function handleCreateAccount(e: React.FormEvent) {
    e.preventDefault();
    if (!username.trim() || password.length < 8) {
      toast.error("Pick a username and a password of at least 8 characters.");
      return;
    }
    setBusy(true);
    try {
      const ep = trimSlash(endpoint);
      const pair = await signup({ endpoint: ep, username: username.trim(), password });
      session.set({
        endpoint: ep,
        token: pair.access_token,
        refreshToken: pair.refresh_token,
        accessExpiresAt: pair.access_expires_at,
        user: pair.user,
        database: session.database || "demo",
      });
      setStep("engine");
    } catch (err) {
      toast.error((err as Error).message || "Could not create the account");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="dark relative flex min-h-screen w-full overflow-hidden bg-[#1a1b1f] font-sans text-[#dfdfd6]">
      <WizardStyles />

      {/* ── Left: brand panel ─────────────────────────────────────────── */}
      <aside className="relative hidden w-[42%] shrink-0 flex-col justify-between overflow-hidden border-r border-[#dfdfd6]/[0.07] p-12 lg:flex">
        <Aurora />
        <div className="relative z-10 flex items-center gap-3 ky-rise" style={{ animationDelay: "40ms" }}>
          <KymaMark className="h-9 w-9" />
          <span className="font-mono text-[17px] font-semibold tracking-[0.18em] text-[#e8e6e0]">
            kyma
          </span>
        </div>

        <div className="relative z-10 max-w-sm ky-rise" style={{ animationDelay: "120ms" }}>
          <div className="mb-4 inline-flex items-center gap-2 rounded-full border border-[#7ed957]/20 bg-[#7ed957]/[0.07] px-3 py-1 font-mono text-[11px] tracking-wider text-[#7ed957]">
            <span className="h-1.5 w-1.5 rounded-full bg-[#7ed957] ky-pulse" />
            FIRST-RUN SETUP
          </div>
          <h1 className="font-mono text-[2.6rem] font-bold leading-[1.04] tracking-tight text-[#e8e6e0]">
            Give the agent
            <br />a database.
          </h1>
          <p className="mt-5 text-[15px] leading-relaxed text-[#dfdfd6]/55">
            Logs, traces, metrics, code and memory in one queryable graph — and an
            agent that can actually think over it. Let's bring your instance to life.
          </p>
        </div>

        <nav className="relative z-10 flex flex-col gap-0.5 ky-rise" style={{ animationDelay: "200ms" }}>
          {STEPS.map((s, i) => {
            const state = i < stepIndex ? "done" : i === stepIndex ? "active" : "todo";
            return (
              <div key={s.id} className="flex items-center gap-3 py-1.5">
                <span
                  className={
                    "grid h-7 w-7 place-items-center rounded-full border font-mono text-[11px] transition-all duration-500 " +
                    (state === "done"
                      ? "border-[#7ed957]/50 bg-[#7ed957]/15 text-[#7ed957]"
                      : state === "active"
                        ? "border-[#7ed957] bg-[#7ed957] text-[#0d1015] ky-glow"
                        : "border-[#dfdfd6]/12 bg-[#dfdfd6]/[0.02] text-[#dfdfd6]/35")
                  }
                >
                  {state === "done" ? <Check className="h-3.5 w-3.5" /> : i + 1}
                </span>
                <span
                  className={
                    "font-mono text-[12.5px] tracking-wide transition-colors duration-500 " +
                    (state === "todo" ? "text-[#dfdfd6]/35" : "text-[#dfdfd6]/90")
                  }
                >
                  {s.label}
                </span>
              </div>
            );
          })}
        </nav>
      </aside>

      {/* ── Right: step content ───────────────────────────────────────── */}
      <main className="relative z-10 flex flex-1 items-center justify-center px-6 py-12 sm:px-12">
        {/* mobile wordmark */}
        <div className="absolute left-6 top-6 flex items-center gap-2 lg:hidden">
          <KymaMark className="h-7 w-7" />
          <span className="font-mono text-sm font-semibold tracking-[0.18em] text-[#e8e6e0]">kyma</span>
        </div>

        {checking ? (
          <Loader2 className="h-6 w-6 animate-spin" style={{ color: GREEN }} />
        ) : (
          <div key={step} className="w-full max-w-md ky-rise">
            {step === "account" && (
              <StepShell
                eyebrow="Step 2 / Account"
                title="Create your admin"
                subtitle="This is the owner account for your kyma instance — and the login your CLI authenticates with."
              >
                <form onSubmit={handleCreateAccount} className="space-y-4">
                  <Field label="Server URL">
                    <Input
                      className={inputCls}
                      value={endpoint}
                      onChange={(e) => setEndpoint(e.target.value)}
                      placeholder="http://localhost:8080"
                      autoComplete="url"
                    />
                  </Field>
                  <Field label="Username">
                    <Input
                      className={inputCls}
                      value={username}
                      onChange={(e) => setUsername(e.target.value)}
                      placeholder="admin"
                      autoComplete="username"
                      autoFocus
                    />
                  </Field>
                  <Field label="Password" hint="At least 8 characters">
                    <Input
                      className={inputCls}
                      type="password"
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      placeholder="••••••••"
                      autoComplete="new-password"
                    />
                  </Field>
                  <PrimaryButton type="submit" busy={busy}>
                    Create account
                  </PrimaryButton>
                </form>
              </StepShell>
            )}

            {step === "agent" && (
              <AgentStep endpoint={trimSlash(endpoint)} onNext={() => setStep("account")} />
            )}
            {step === "engine" && (
              <EngineStep endpoint={trimSlash(endpoint)} onNext={() => setStep("data")} />
            )}
            {step === "data" && (
              <DataStep endpoint={trimSlash(endpoint)} onNext={() => setStep("done")} />
            )}
            {step === "done" && <DoneStep onEnter={enterApp} />}
          </div>
        )}
      </main>
    </div>
  );
}

/* ─────────────────────────────────────────────────────────────────────────
 * Engine step
 * ───────────────────────────────────────────────────────────────────────── */

const PROVIDERS: { kind: EngineKind; name: string; tag: string; logo?: string }[] = [
  { kind: "ollama", name: "Ollama", tag: "Local · private", logo: "/icons/brand/ollama.svg" },
  { kind: "anthropic", name: "Anthropic", tag: "Claude models", logo: "/icons/brand/anthropic.svg" },
  { kind: "openai", name: "OpenAI", tag: "GPT models", logo: "/icons/brand/openai.svg" },
  { kind: "claude_cli", name: "Claude Code", tag: "Local CLI" },
];

const FALLBACK_MODELS: Record<string, string[]> = {
  anthropic: ["claude-sonnet-4-6", "claude-opus-4-1", "claude-haiku-4-5"],
  openai: ["gpt-4o", "gpt-4o-mini", "o3-mini"],
  claude_cli: ["sonnet", "opus", "haiku"],
};

const OLLAMA_HOST = "http://localhost:11434";

function EngineStep({ endpoint, onNext }: { endpoint: string; onNext: () => void }) {
  const session = useSession();
  const args = useMemo(() => ({ endpoint, token: session.token }), [endpoint, session.token]);
  const [probe, setProbe] = useState<SetupProbe | null>(null);
  const [engines, setEngines] = useState<EngineList | null>(null);
  const [provider, setProvider] = useState<EngineKind>("ollama");
  const [model, setModel] = useState("gemma4:latest");
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [rechecking, setRechecking] = useState(false);

  async function refresh() {
    const [p, e] = await Promise.allSettled([getSetupProbe(endpoint), listEngines(args)]);
    if (p.status === "fulfilled") setProbe(p.value);
    if (e.status === "fulfilled") setEngines(e.value);
  }
  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const ollamaModels = probe?.ollama_models ?? [];
  const models = useMemo(() => {
    if (provider === "ollama") return Array.from(new Set(["gemma4:latest", ...ollamaModels]));
    const fromServer = engines?.available.find((a) => a.kind === provider)?.models;
    return fromServer?.length ? fromServer : FALLBACK_MODELS[provider] ?? [];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider, engines, ollamaModels.join(",")]);

  useEffect(() => {
    setModel(provider === "ollama" ? "gemma4:latest" : models[0] ?? "");
    setApiKey("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider]);

  const keyDetected =
    provider === "anthropic"
      ? !!probe?.anthropic_key_present
      : provider === "openai"
        ? !!probe?.openai_key_present
        : false;
  const needsKey =
    (provider === "anthropic" || provider === "openai") && !keyDetected && !apiKey.trim();
  const ollamaReachable = !!probe?.ollama_reachable;
  const modelInstalled = ollamaModels.includes(model);
  const claudeReady = !!probe?.claude_binary_found;
  const blocked =
    busy ||
    (provider === "ollama" && !ollamaReachable) ||
    (provider === "claude_cli" && !claudeReady) ||
    needsKey;

  async function recheck() {
    setRechecking(true);
    await refresh();
    setRechecking(false);
  }

  async function save() {
    setBusy(true);
    try {
      let credential_id: string | null = null;
      if ((provider === "anthropic" || provider === "openai") && apiKey.trim()) {
        const cred = await createCredential({
          ...args,
          body: { label: `${provider}-api-key`, value: { kind: "api_key", value: apiKey.trim() } },
        });
        credential_id = cred.id;
      }
      const cfg: EngineConfig = {
        kind: provider,
        model,
        credential_id,
        host: provider === "ollama" ? OLLAMA_HOST : null,
      };
      await putEngine(args, cfg);
      onNext();
    } catch (err) {
      toast.error((err as Error).message || "Could not save the engine");
    } finally {
      setBusy(false);
    }
  }

  return (
    <StepShell
      eyebrow="Step 3 / AI engine"
      title="Set up your engine"
      subtitle="Pick a provider and model. On-device Ollama keeps everything private — cloud providers just need a key."
    >
      <div className="grid grid-cols-2 gap-2.5">
        {PROVIDERS.map((p) => {
          const ready =
            p.kind === "ollama"
              ? probe?.ollama_reachable
              : p.kind === "anthropic"
                ? probe?.anthropic_key_present
                : p.kind === "openai"
                  ? probe?.openai_key_present
                  : probe?.claude_binary_found;
          return (
            <ProviderTile
              key={p.kind}
              active={provider === p.kind}
              onClick={() => setProvider(p.kind)}
              logo={p.logo}
              name={p.name}
              tag={p.tag}
              ready={!!ready}
            />
          );
        })}
      </div>

      <div className="mt-3.5 space-y-3 rounded-xl border border-[#dfdfd6]/[0.08] bg-[#dfdfd6]/[0.015] p-4">
        <Field label="Model">
          <BrandSelect value={model} onChange={setModel} options={models} />
        </Field>

        {provider === "ollama" &&
          (!ollamaReachable ? (
            <Notice tone="warn" title="Ollama isn't running on this host">
              Install it, start the daemon, then re-check.
              <div className="mt-2 space-y-1.5">
                <CopyRow text="brew install ollama   # or see ollama.com/download" />
                <CopyRow text="ollama serve" />
              </div>
              <RecheckButton onClick={recheck} busy={rechecking} />
            </Notice>
          ) : modelInstalled ? (
            <Notice tone="ok" title={`${model} is installed and ready.`} />
          ) : (
            <Notice tone="warn" title={`${model} isn't pulled yet`}>
              Pull it once (it's then cached locally), then re-check:
              <div className="mt-2">
                <CopyRow text={`ollama pull ${model}`} />
              </div>
              <RecheckButton onClick={recheck} busy={rechecking} />
            </Notice>
          ))}

        {(provider === "anthropic" || provider === "openai") &&
          (keyDetected ? (
            <Notice
              tone="ok"
              title={`${provider === "anthropic" ? "ANTHROPIC_API_KEY" : "OPENAI_API_KEY"} detected — ready.`}
            />
          ) : (
            <Field label="API key" hint="stored encrypted in kyma">
              <Input
                className={inputCls}
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={provider === "anthropic" ? "sk-ant-…" : "sk-…"}
                autoComplete="off"
              />
            </Field>
          ))}

        {provider === "claude_cli" &&
          (claudeReady ? (
            <Notice tone="ok" title="Claude Code CLI detected — uses your local login, skills & MCPs." />
          ) : (
            <Notice tone="warn" title="Claude Code isn't installed">
              Install it from claude.com/code, then re-check.
              <RecheckButton onClick={recheck} busy={rechecking} />
            </Notice>
          ))}
      </div>

      <PrimaryButton onClick={save} busy={busy} disabled={blocked} className="mt-4">
        Use this engine
      </PrimaryButton>
    </StepShell>
  );
}

/* ─────────────────────────────────────────────────────────────────────────
 * Data step
 * ───────────────────────────────────────────────────────────────────────── */

function DataStep({ endpoint, onNext }: { endpoint: string; onNext: () => void }) {
  const session = useSession();
  const [busy, setBusy] = useState(false);

  async function loadSample() {
    setBusy(true);
    try {
      await ingestSample({ endpoint, token: session.token, database: "demo", table: "otel_logs" });
      session.set({ database: "demo" });
      toast.success("Sample telemetry loaded into demo.otel_logs");
      onNext();
    } catch (err) {
      toast.error((err as Error).message || "Could not load sample data");
    } finally {
      setBusy(false);
    }
  }

  return (
    <StepShell
      eyebrow="Step 4 / Data"
      title="Bring in your data"
      subtitle="The quickest way to connect real sources is the CLI — or just ask your coding agent to do it for you."
    >
      <div className="space-y-3.5">
        {/* Recommended: CLI */}
        <div>
          <div className="mb-1.5 flex items-center gap-2 font-mono text-[11px] uppercase tracking-wider text-[#dfdfd6]/40">
            <Terminal className="h-3.5 w-3.5" /> Connect a source from the CLI
          </div>
          <CopyRow text="kyma connector add github <owner>/<repo> --start" />
          <p className="mt-1.5 text-[12px] leading-snug text-[#dfdfd6]/45">
            GitHub, GitLab, Bitbucket, Postgres, Prometheus &amp; more — see{" "}
            <span className="font-mono text-[#dfdfd6]/70">kyma connector --help</span>.
          </p>
        </div>

        {/* Or: let the coding agent ingest */}
        <div className="rounded-xl border border-[#7ed957]/25 bg-[#7ed957]/[0.05] p-4">
          <div className="mb-2 flex items-center gap-2 font-mono text-[11px] uppercase tracking-wider text-[#7ed957]">
            <Sparkles className="h-3.5 w-3.5" /> Or let your coding agent do it
          </div>
          <p className="mb-2 text-[12.5px] leading-snug text-[#dfdfd6]/65">
            Your coding agent already has the kyma skill — just ask it:
          </p>
          <CopyRow prefix="› " text="Discover the data sources I can reach and ingest them into kyma." />
        </div>

        {/* Secondary: quick sample */}
        <button
          type="button"
          onClick={loadSample}
          disabled={busy}
          className="flex w-full items-center justify-between rounded-lg border border-[#dfdfd6]/[0.08] bg-[#dfdfd6]/[0.02] px-4 py-3 text-left transition-all hover:border-[#dfdfd6]/15 disabled:opacity-60"
        >
          <span className="flex items-center gap-2.5 text-[13px] text-[#dfdfd6]/80">
            {busy ? (
              <Loader2 className="h-4 w-4 animate-spin text-[#7ed957]" />
            ) : (
              <Database className="h-4 w-4 text-[#7ed957]" />
            )}
            Just exploring? Load 40 rows of sample telemetry
          </span>
          <ArrowRight className="h-4 w-4 shrink-0 text-[#dfdfd6]/35" />
        </button>
      </div>

      <PrimaryButton onClick={onNext} className="mt-5">
        Continue
      </PrimaryButton>
    </StepShell>
  );
}

/* ─────────────────────────────────────────────────────────────────────────
 * Coding-agent skills step
 * ───────────────────────────────────────────────────────────────────────── */

function AgentStep({ endpoint, onNext }: { endpoint: string; onNext: () => void }) {
  const commands = [
    { label: "1 / Install the kyma CLI", cmd: "cargo install kyma-cli   # or: brew install kyma" },
    { label: "2 / Authenticate to this instance", cmd: `kyma connect ${endpoint}` },
    { label: "3 / Add the skill to your coding agent", cmd: "kyma install-skill --also-link-claude" },
  ];
  return (
    <StepShell
      eyebrow="Step 1 / Coding agent"
      title="Connect your coding agent"
      subtitle="Start here: install the kyma CLI, authenticate, and drop the kyma skill into Claude Code, Cursor or any coding agent so it can query your data through the CLI."
    >
      <div className="space-y-3">
        {commands.map((c) => (
          <div key={c.label}>
            <div className="mb-1.5 flex items-center gap-2 font-mono text-[11px] uppercase tracking-wider text-[#dfdfd6]/40">
              <Terminal className="h-3.5 w-3.5" />
              {c.label}
            </div>
            <CopyRow text={c.cmd} />
          </div>
        ))}
      </div>
      <div className="mt-4 flex items-start gap-2 rounded-lg border border-[#dfdfd6]/[0.08] bg-[#dfdfd6]/[0.02] p-3 text-[12px] leading-snug text-[#dfdfd6]/55">
        <KeyRound className="mt-0.5 h-4 w-4 shrink-0 text-[#7ed957]/70" />
        Your agent then runs <span className="font-mono text-[#dfdfd6]/80">kyma query "…"</span> to ask
        questions over everything you've ingested.
      </div>
      <PrimaryButton onClick={onNext} className="mt-5">
        Continue
      </PrimaryButton>
    </StepShell>
  );
}

/* ─────────────────────────────────────────────────────────────────────────
 * Done step
 * ───────────────────────────────────────────────────────────────────────── */

function DoneStep({ onEnter }: { onEnter: () => void }) {
  return (
    <div className="text-center">
      <div className="mx-auto mb-7 ky-rise">
        <KymaMark className="mx-auto h-16 w-16 ky-glow rounded-full" />
      </div>
      <div className="mb-1 font-mono text-[11px] uppercase tracking-[0.18em] text-[#7ed957]">
        Ready
      </div>
      <h2 className="font-mono text-2xl font-bold tracking-tight text-[#e8e6e0]">You're all set</h2>
      <p className="mx-auto mt-3 max-w-sm text-[14px] leading-relaxed text-[#dfdfd6]/55">
        Your instance is live with an AI engine, data and a coding-agent skill. Time to give the agent a
        database.
      </p>
      <PrimaryButton onClick={onEnter} className="mt-7">
        Enter kyma
        <ArrowRight className="ml-1 h-4 w-4" />
      </PrimaryButton>
    </div>
  );
}

/* ─────────────────────────────────────────────────────────────────────────
 * Shared building blocks
 * ───────────────────────────────────────────────────────────────────────── */

function StepShell({
  eyebrow,
  title,
  subtitle,
  children,
}: {
  eyebrow: string;
  title: string;
  subtitle: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="mb-1.5 font-mono text-[11px] uppercase tracking-[0.18em] text-[#7ed957]">
        {eyebrow}
      </div>
      <h2 className="font-mono text-[1.7rem] font-bold leading-tight tracking-tight text-[#e8e6e0]">
        {title}
      </h2>
      <p className="mt-2 text-[14px] leading-relaxed text-[#dfdfd6]/55">{subtitle}</p>
      <div className="mt-6">{children}</div>
    </div>
  );
}

function ProviderTile({
  active,
  onClick,
  logo,
  name,
  tag,
  ready,
}: {
  active: boolean;
  onClick: () => void;
  logo?: string;
  name: string;
  tag: string;
  ready: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={
        "relative flex items-center gap-3 rounded-xl border p-3 text-left transition-all duration-200 " +
        (active
          ? "border-[#7ed957]/70 bg-[#7ed957]/[0.08]"
          : "border-[#dfdfd6]/[0.08] bg-[#dfdfd6]/[0.02] hover:border-[#dfdfd6]/15")
      }
    >
      <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-[#dfdfd6]/[0.08]">
        {logo ? (
          <img src={logo} alt={name} className="h-5 w-5 object-contain" />
        ) : (
          <Terminal className="h-5 w-5 text-[#dfdfd6]/80" />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-[13.5px] font-medium text-[#e8e6e0]">{name}</span>
          {ready && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-[#7ed957]" />}
        </span>
        <span className="block truncate text-[11px] text-[#dfdfd6]/45">{tag}</span>
      </span>
    </button>
  );
}

function BrandSelect({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: string[];
}) {
  return (
    <div className="relative">
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-11 w-full appearance-none rounded-lg border border-[#dfdfd6]/12 bg-[#111317] px-3 pr-9 font-mono text-[13px] text-[#e8e6e0] focus:outline-none focus:ring-1 focus:ring-[#7ed957]/60"
      >
        {options.length === 0 && <option value="">—</option>}
        {options.map((o) => (
          <option key={o} value={o} className="bg-[#111317]">
            {o}
          </option>
        ))}
      </select>
      <ChevronDown className="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[#dfdfd6]/40" />
    </div>
  );
}

function Notice({
  tone,
  title,
  children,
}: {
  tone: "ok" | "warn";
  title: string;
  children?: React.ReactNode;
}) {
  const ok = tone === "ok";
  return (
    <div
      className={
        "rounded-lg border p-3 text-[12.5px] leading-snug " +
        (ok
          ? "border-[#7ed957]/25 bg-[#7ed957]/[0.06] text-[#dfdfd6]/80"
          : "border-amber-400/20 bg-amber-400/[0.05] text-[#dfdfd6]/75")
      }
    >
      <div className="flex items-center gap-2 font-medium text-[#e8e6e0]">
        {ok ? (
          <Check className="h-4 w-4 text-[#7ed957]" />
        ) : (
          <AlertCircle className="h-4 w-4 text-amber-400/90" />
        )}
        {title}
      </div>
      {children && <div className="mt-1.5 text-[#dfdfd6]/65">{children}</div>}
    </div>
  );
}

function RecheckButton({ onClick, busy }: { onClick: () => void; busy?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      className="mt-2.5 inline-flex items-center gap-1.5 rounded-md border border-[#dfdfd6]/15 bg-[#dfdfd6]/[0.03] px-2.5 py-1.5 font-mono text-[11px] text-[#dfdfd6]/80 transition-colors hover:border-[#7ed957]/40 hover:text-[#e8e6e0] disabled:opacity-60"
    >
      <RefreshCw className={"h-3.5 w-3.5 " + (busy ? "animate-spin" : "")} />
      Re-check
    </button>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline justify-between">
        <Label className="text-[13px] text-[#dfdfd6]/80">{label}</Label>
        {hint && <span className="font-mono text-[11px] text-[#dfdfd6]/35">{hint}</span>}
      </div>
      {children}
    </div>
  );
}

function PrimaryButton({
  children,
  busy,
  className,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { busy?: boolean }) {
  return (
    <Button
      {...props}
      disabled={busy || props.disabled}
      className={
        "h-11 w-full rounded-lg bg-[#7ed957] text-[14px] font-semibold text-[#0d1015] " +
        "shadow-[0_10px_34px_-14px_rgba(126,217,87,0.8)] transition-all hover:bg-[#92e36f] " +
        "disabled:opacity-70 " +
        (className ?? "")
      }
    >
      {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : children}
    </Button>
  );
}

function CopyRow({ text, prefix = "$ " }: { text: string; prefix?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={() => {
        navigator.clipboard?.writeText(text).then(
          () => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1400);
          },
          () => toast.error("Could not copy"),
        );
      }}
      className="group flex w-full items-center justify-between gap-3 rounded-lg border border-[#dfdfd6]/[0.08] bg-[#0d1015] px-3.5 py-2.5 text-left transition-colors hover:border-[#7ed957]/30"
    >
      <code className="truncate font-mono text-[12.5px] text-[#dfdfd6]/85">
        <span className="text-[#7ed957]/70">{prefix}</span>
        {text}
      </code>
      <span className="shrink-0 text-[#dfdfd6]/40 transition-colors group-hover:text-[#dfdfd6]/80">
        {copied ? <Check className="h-4 w-4 text-[#7ed957]" /> : <Copy className="h-4 w-4" />}
      </span>
    </button>
  );
}

/** The real kyma mark — phosphor-ring aperture (from the kyma docs site). */
function KymaMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 96 96" role="img" aria-label="kyma" className={className}>
      <defs>
        <radialGradient id="kymaCore" cx="50%" cy="50%" r="50%">
          <stop offset="0%" stopColor="#0d1015" />
          <stop offset="80%" stopColor="#0d1015" />
          <stop offset="100%" stopColor="#11151b" />
        </radialGradient>
        <linearGradient id="kymaRing" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="#7ed957" />
          <stop offset="60%" stopColor="#7ed957" stopOpacity="0.5" />
          <stop offset="100%" stopColor="#7ed957" stopOpacity="0.15" />
        </linearGradient>
      </defs>
      <circle cx="48" cy="48" r="44" fill="none" stroke="url(#kymaRing)" strokeWidth="2" />
      <circle cx="48" cy="48" r="36" fill="url(#kymaCore)" stroke="#2a2e36" strokeWidth="1" />
      <circle cx="48" cy="48" r="36" fill="none" stroke="#7ed957" strokeWidth="0.5" opacity="0.4" />
      <g stroke="#e8e6e0" strokeWidth="3.5" strokeLinecap="round" fill="none">
        <line x1="34" y1="32" x2="34" y2="64" />
        <line x1="34" y1="48" x2="58" y2="34" />
        <line x1="34" y1="48" x2="58" y2="62" />
      </g>
      <circle cx="58" cy="34" r="2.5" fill="#7ed957" />
      <circle cx="58" cy="62" r="2.5" fill="#7ed957" />
    </svg>
  );
}

function Aurora() {
  return (
    <div className="pointer-events-none absolute inset-0 overflow-hidden">
      <div className="ky-blob absolute -left-1/4 top-[-12%] h-[58%] w-[70%] rounded-full bg-[#7ed957]/[0.14] blur-[100px]" />
      <div
        className="ky-blob absolute right-[-18%] top-[24%] h-[52%] w-[58%] rounded-full bg-[#34d399]/[0.10] blur-[100px]"
        style={{ animationDelay: "-7s", animationDuration: "28s" }}
      />
      <div
        className="ky-blob absolute bottom-[-16%] left-[8%] h-[52%] w-[62%] rounded-full bg-[#2dd4bf]/[0.08] blur-[110px]"
        style={{ animationDelay: "-13s", animationDuration: "32s" }}
      />
      <div className="ky-grid absolute inset-0 opacity-[0.16]" />
      <div className="ky-scan absolute inset-0 opacity-[0.05]" />
    </div>
  );
}

function WizardStyles() {
  return (
    <style>{`
      @keyframes ky-rise { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: none; } }
      .ky-rise { animation: ky-rise 0.6s cubic-bezier(0.22,1,0.36,1) both; }
      @keyframes ky-blob { 0%,100% { transform: translate(0,0) scale(1); } 33% { transform: translate(6%,4%) scale(1.12); } 66% { transform: translate(-5%,-3%) scale(0.95); } }
      .ky-blob { animation: ky-blob 24s ease-in-out infinite; }
      @keyframes ky-glow { 0%,100% { filter: drop-shadow(0 0 0 rgba(126,217,87,0.0)); } 50% { filter: drop-shadow(0 0 10px rgba(126,217,87,0.55)); } }
      .ky-glow { animation: ky-glow 3.6s ease-in-out infinite; }
      @keyframes ky-pulse { 0%,100% { opacity: 1; box-shadow: 0 0 0 0 rgba(126,217,87,0.6); } 50% { opacity: 0.6; box-shadow: 0 0 8px 2px rgba(126,217,87,0.5); } }
      .ky-pulse { animation: ky-pulse 2.2s ease-in-out infinite; }
      .ky-grid {
        background-image: linear-gradient(rgba(126,217,87,0.10) 1px, transparent 1px),
          linear-gradient(90deg, rgba(126,217,87,0.10) 1px, transparent 1px);
        background-size: 38px 38px;
        mask-image: radial-gradient(ellipse 70% 60% at 28% 42%, #000 25%, transparent 72%);
      }
      .ky-scan { background-image: repeating-linear-gradient(0deg, rgba(255,255,255,0.5) 0px, rgba(255,255,255,0.5) 1px, transparent 1px, transparent 3px); }
      @media (prefers-reduced-motion: reduce) {
        .ky-rise, .ky-blob, .ky-glow, .ky-pulse { animation: none !important; }
      }
    `}</style>
  );
}
