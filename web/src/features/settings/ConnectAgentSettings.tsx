import { useState } from "react";
import { Check, Copy, Terminal } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { useSession } from "@/sdk/session";
import { cn } from "@/lib/utils";

/**
 * Settings → Connect agent.
 *
 * Walks the user through installing the kyma CLI on their host and wiring
 * it up to this server so any coding agent (Claude Code, Cursor, Aider)
 * can query Kyma directly via the installed skill.
 *
 * Each step is a labelled snippet with a Copy button; the server URL and
 * (when available) the bearer token are pre-substituted from the active
 * session so the user doesn't have to edit before pasting.
 */
export function ConnectAgentSettings() {
  const { endpoint, token } = useSession();

  // Reasonable fallback so the page is still useful when the user lands
  // here before logging in (rare in practice — the route is gated — but
  // the snippet block shouldn't look broken).
  const url = endpoint || "http://localhost:8080";
  const tokenPart = token
    ? ` --token "${token}"`
    : ` --token "<paste your bearer token here>"`;

  const installCmd = "cargo install --path crates/kyma-cli";
  const connectCmd = `kyma connect ${url}${tokenPart}`;
  const skillCmd = "kyma install-skill --also-link-claude";
  const tryCmd = 'kyma query "What databases do we have?"';

  return (
    <Card>
      <CardContent className="space-y-4 p-5">
        <p className="text-sm text-muted-foreground">
          Install the <code className="font-mono text-foreground">kyma</code>{" "}
          CLI on your dev host and any coding agent (Claude Code, Cursor,
          Aider, Codex …) can query this server in real time — the agent
          discovers the installed skill automatically and shells out to
          <code className="font-mono text-foreground"> kyma query</code> on
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
            token
              ? "Your bearer token is pre-filled below — paste and run."
              : "Sign in first so we can pre-fill the bearer token, or paste your own."
          }
          cmd={connectCmd}
          secret={Boolean(token)}
        />

        <Step
          n={3}
          title="Install the Kyma skill"
          hint="Writes ~/.kyma/skills/kyma/SKILL.md and (with --also-link-claude) symlinks it into ~/.claude/skills/kyma so Claude Code discovers it on next launch."
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
              Code) or <code className="font-mono">~/.kyma/skills/</code> (any
              skill-aware agent) sees a SKILL.md whose frontmatter says
              "when the user asks about their data, use the kyma CLI."
            </p>
            <p>
              The agent then issues{" "}
              <code className="font-mono">kyma query "…"</code> as a Bash
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
}: {
  n: number;
  title: string;
  hint: string;
  cmd: string;
  secret?: boolean;
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
