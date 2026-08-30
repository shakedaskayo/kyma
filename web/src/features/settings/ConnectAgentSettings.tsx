import { useState } from "react";
import { Check, Copy, KeyRound, Loader2, Terminal } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { useSession } from "@/sdk/session";
import { createApiToken } from "@/sdk/tokens";
import { cn } from "@/lib/utils";

/**
 * Settings → Connect agent.
 *
 * Walks the user through installing the pensieve CLI on their host and wiring
 * it up to this server so any coding agent (Claude Code, Cursor, Aider)
 * can query Pensieve directly via the installed skill.
 *
 * Each step is a labelled snippet with a Copy button; the server URL is
 * pre-substituted from the active session, and the bearer token is a
 * long-lived API token minted on demand — NOT the browser session token,
 * which expires within the hour and would silently break the CLI and
 * every capture hook pointed at it.
 */
export function ConnectAgentSettings() {
  const { endpoint, token } = useSession();
  const [minted, setMinted] = useState<string | null>(null);
  const [minting, setMinting] = useState(false);
  const [mintError, setMintError] = useState<string | null>(null);

  function mint() {
    setMinting(true);
    setMintError(null);
    createApiToken({ name: "cli-connect" })
      .then((t) => setMinted(t.token))
      .catch((e: unknown) => setMintError((e as Error).message || "couldn't mint a token"))
      .finally(() => setMinting(false));
  }

  // Reasonable fallback so the page is still useful when the user lands
  // here before logging in (rare in practice — the route is gated — but
  // the snippet block shouldn't look broken).
  const url = endpoint || "http://localhost:8080";
  const tokenPart = minted
    ? ` --token "${minted}"`
    : ` --token "<generate a token above, or paste your own>"`;

  const installCmd = "cargo install --path crates/pensieve-cli";
  const connectCmd = `pensieve connect ${url}${tokenPart}`;
  const skillCmd = "pensieve install-skill --also-link-claude";
  const tryCmd = 'pensieve query "What databases do we have?"';

  return (
    <Card>
      <CardContent className="space-y-4 p-5">
        <p className="text-sm text-muted-foreground">
          Install the <code className="font-mono text-foreground">pensieve</code>{" "}
          CLI on your dev host and any coding agent (Claude Code, Cursor,
          Aider, Codex …) can query this server in real time — the agent
          discovers the installed skill automatically and shells out to
          <code className="font-mono text-foreground"> pensieve query</code> on
          demand.
        </p>

        <Step
          n={1}
          title="Install the CLI"
          hint="Builds from source. Requires Rust ≥1.74."
          cmd={installCmd}
        />

        <Step
          n={2}
          title="Connect this server"
          hint={
            minted
              ? "Long-lived API token minted and pre-filled below — paste and run."
              : "Mint a long-lived API token for the CLI. (Browser session tokens expire within the hour and would break the connection silently.)"
          }
          cmd={connectCmd}
          secret={Boolean(minted)}
          action={
            !minted ? (
              <div className="ml-7 flex items-center gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1.5 text-xs"
                  disabled={minting || !token}
                  onClick={mint}
                >
                  {minting ? (
                    <Loader2 className="h-3 w-3 animate-spin" />
                  ) : (
                    <KeyRound className="h-3 w-3" />
                  )}
                  Generate CLI token
                </Button>
                {mintError && (
                  <span className="text-xs text-destructive">{mintError}</span>
                )}
              </div>
            ) : undefined
          }
        />

        <Step
          n={3}
          title="Install the Pensieve skill"
          hint="Writes ~/.pensieve/skills/pensieve/SKILL.md and (with --also-link-claude) symlinks it into ~/.claude/skills/pensieve so Claude Code discovers it on next launch."
          cmd={skillCmd}
        />

        <Step
          n={4}
          title="Try it"
          hint="Streams the agent's answer to stdout. Coding agents pipe through this same surface."
          cmd={tryCmd}
        />

        <details className="rounded-md border bg-muted/20 p-3 text-xs text-muted-foreground">
          <summary className="cursor-pointer select-none font-medium text-foreground">
            How the coding agent discovers the skill
          </summary>
          <div className="mt-2 space-y-2">
            <p>
              After step 3, every coding agent that walks{" "}
              <code className="font-mono">~/.claude/skills/</code> (Claude
              Code) or <code className="font-mono">~/.pensieve/skills/</code> (any
              skill-aware agent) sees a SKILL.md whose frontmatter says
              "when the user asks about their data, use the pensieve CLI."
            </p>
            <p>
              The agent then issues{" "}
              <code className="font-mono">pensieve query "…"</code> as a Bash
              call and gets streaming answers back. No MCP server, no extra
              wiring — the agent uses the CLI like any other Unix tool.
            </p>
          </div>
        </details>
      </CardContent>
    </Card>
  );
}

function Step({
  n,
  title,
  hint,
  cmd,
  secret = false,
  action,
}: {
  n: number;
  title: string;
  hint: string;
  cmd: string;
  secret?: boolean;
  action?: React.ReactNode;
}) {
  const [copied, setCopied] = useState(false);
  const [revealed, setRevealed] = useState(false);

  function copy() {
    navigator.clipboard
      .writeText(cmd)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {
        // best-effort; some browsers gate clipboard on https
      });
  }

  // Mask the token in the displayed command unless the user clicks
  // "reveal". Always copy the full unmasked command (the user wants to
  // paste a working command, not a redacted one).
  const display = secret && !revealed ? maskToken(cmd) : cmd;

  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline gap-2">
        <span
          className={cn(
            "inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-primary/10",
            "text-[10px] font-semibold text-primary",
          )}
        >
          {n}
        </span>
        <span className="text-sm font-medium">{title}</span>
      </div>
      <p className="pl-7 text-xs text-muted-foreground">{hint}</p>
      {action}
      <div className="relative ml-7 overflow-hidden rounded-md border bg-background">
        <div className="absolute left-2 top-1.5 text-muted-foreground">
          <Terminal className="h-3 w-3" />
        </div>
        <pre className="overflow-x-auto px-7 py-1.5 text-[12px] leading-relaxed">
          <code className="font-mono">{display}</code>
        </pre>
        <div className="absolute right-1 top-1 flex items-center gap-0.5">
          {secret && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-6 px-1.5 text-[10px] text-muted-foreground hover:text-foreground"
              onClick={() => setRevealed((v) => !v)}
            >
              {revealed ? "hide" : "reveal"}
            </Button>
          )}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 px-1.5"
            onClick={copy}
            aria-label="Copy command"
          >
            {copied ? (
              <Check className="h-3 w-3 text-emerald-500" />
            ) : (
              <Copy className="h-3 w-3 text-muted-foreground" />
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}

function maskToken(cmd: string): string {
  // Replace anything that looks like a bearer token inside --token "..."
  // with a fixed-length filler so the layout doesn't reshuffle on
  // reveal/hide.
  return cmd.replace(/--token\s+"[^"]*"/, '--token "•••••••••••••••••"');
}
