import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, Sparkles } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { useSession } from "@/sdk/session";
import {
  listSkills,
  putEnabledSkills,
  type SkillSource,
} from "@/sdk/agent-skills";

/**
 * Settings → Skills card.
 *
 * Lists every skill discovered on the host (project + user + plugin dirs).
 * The user toggles the checkbox to enable a skill; on Save, the enabled
 * set is persisted via PUT /v1/agent/skills/enabled. Enabled skills are
 * injected into the agent's system prompt for non-Claude-CLI engines —
 * the Claude CLI engine loads them natively.
 */
export function SkillsSettings() {
  const { endpoint, token } = useSession();
  const qc = useQueryClient();
  const enabled = Boolean(endpoint && token);

  const skills = useQuery({
    queryKey: ["skills", endpoint],
    queryFn: () => listSkills({ endpoint, token }),
    enabled,
  });

  const [draft, setDraft] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");

  // Re-seed from server whenever the discovered list changes.
  useEffect(() => {
    if (skills.data) {
      setDraft(new Set(skills.data.filter((s) => s.enabled).map((s) => s.name)));
    }
  }, [skills.data]);

  const filtered = useMemo(() => {
    const data = skills.data ?? [];
    if (!filter.trim()) return data;
    const q = filter.trim().toLowerCase();
    return data.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.path.toLowerCase().includes(q),
    );
  }, [skills.data, filter]);

  const save = useMutation({
    mutationFn: () => putEnabledSkills({ endpoint, token }, Array.from(draft)),
    onSuccess: () => {
      toast.success(`Skills saved (${draft.size} enabled)`);
      qc.invalidateQueries({ queryKey: ["skills", endpoint] });
    },
    onError: (e: Error) => toast.error(`Save failed: ${e.message}`),
  });

  const sourceCounts = useMemo(() => {
    const m: Record<SkillSource, number> = { project: 0, user: 0, plugin: 0 };
    for (const s of skills.data ?? []) m[s.source] += 1;
    return m;
  }, [skills.data]);

  function toggle(name: string) {
    setDraft((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  return (
    <Card>
      <CardContent className="space-y-4 p-4">
        <p className="text-xs text-muted-foreground">
          Skills are markdown files Kyma discovers in{" "}
          <code className="font-mono">~/.claude/skills/</code>, Claude Code
          plugin caches, and{" "}
          <code className="font-mono">./.claude/skills/</code>. Enabled skills
          are injected into the agent's system prompt for Anthropic / OpenAI /
          Ollama engines. The Claude Code CLI engine loads skills natively, so
          toggles here don't apply to it.
        </p>

        <div className="flex items-center justify-between gap-2">
          <input
            type="search"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter skills…"
            className="h-9 flex-1 rounded-md border border-input bg-background px-3 py-1 text-sm"
          />
          <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
            <span>{sourceCounts.project} project</span>
            <span>·</span>
            <span>{sourceCounts.user} user</span>
            <span>·</span>
            <span>{sourceCounts.plugin} plugin</span>
          </div>
        </div>

        {skills.isLoading && (
          <p className="text-sm text-muted-foreground">
            <Loader2 className="mr-1 inline h-3 w-3 animate-spin" />
            Discovering skills…
          </p>
        )}
        {skills.isError && (
          <p className="text-sm text-destructive">
            Failed to load skills: {(skills.error as Error).message}
          </p>
        )}
        {!skills.isLoading && filtered.length === 0 && (
          <p className="text-sm text-muted-foreground">
            <Sparkles className="mr-1 inline h-3 w-3" />
            {skills.data?.length
              ? "No skills match the filter."
              : "No skills found on this host."}
          </p>
        )}

        <ul className="max-h-[420px] space-y-2 overflow-y-auto">
          {filtered.map((s) => (
            <li
              key={s.name}
              className="rounded-md border bg-muted/20 p-3 transition-colors hover:bg-muted/40"
            >
              <label className="flex items-start gap-3">
                <input
                  type="checkbox"
                  checked={draft.has(s.name)}
                  onChange={() => toggle(s.name)}
                  className="mt-1 h-4 w-4 cursor-pointer"
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium">{s.name}</span>
                    <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-muted-foreground">
                      {s.source}
                    </span>
                  </div>
                  {s.description && (
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      {s.description}
                    </p>
                  )}
                  <p
                    className="mt-1 truncate font-mono text-[10px] text-muted-foreground/70"
                    title={s.path}
                  >
                    {s.path}
                  </p>
                </div>
              </label>
            </li>
          ))}
        </ul>

        <div className="flex items-center gap-2 pt-2">
          <Button
            size="sm"
            onClick={() => save.mutate()}
            disabled={save.isPending}
          >
            {save.isPending ? "Saving…" : `Save (${draft.size} enabled)`}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() =>
              setDraft(
                new Set(
                  (skills.data ?? []).filter((s) => s.enabled).map((s) => s.name),
                ),
              )
            }
            disabled={save.isPending}
          >
            Revert
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
