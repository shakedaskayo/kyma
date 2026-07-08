import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Flower2, GitBranch, RefreshCw, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SkeletonRows } from "@/components/ui/skeleton";
import { StatusPill } from "@/components/ui/status";
import { realmsLabel, type BrainRunRecord } from "@/sdk/brains";
import { relTime } from "@/lib/time";
import { CloneInstructions } from "./CloneInstructions";
import {
  useBrain,
  useDeleteBrain,
  useTriggerExport,
  useTriggerGardener,
} from "./useBrains";

function runTone(run: BrainRunRecord): "ok" | "error" | "idle" {
  if (run.error) return "error";
  if (run.noop) return "idle";
  return "ok";
}

function runDetail(run: BrainRunRecord): string {
  if (run.error) return run.error;
  if (run.kind === "push_ingest") return `${run.notes_ingested} notes ingested from push`;
  if (run.noop) return "no changes";
  return `${run.files_written} files${run.commit ? ` · ${run.commit.slice(0, 8)}` : ""}`;
}

export function BrainDetail({ name }: { name: string }) {
  const { data: brain, isLoading, error } = useBrain(name);
  const exportNow = useTriggerExport(name);
  const garden = useTriggerGardener(name);
  const del = useDeleteBrain();
  const navigate = useNavigate();
  const [confirming, setConfirming] = useState(false);

  if (isLoading) return <SkeletonRows rows={4} className="py-2" />;
  if (error || !brain) {
    return (
      <div className="py-10 text-center text-sm text-destructive">
        {(error as Error | undefined)?.message ?? "Brain not found"}
      </div>
    );
  }
  const { config, runtime } = brain;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <span className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10 text-primary ring-1 ring-inset ring-primary/15">
          <GitBranch className="h-4.5 w-4.5" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="text-base font-semibold">{config.name}</h2>
            <Badge variant="outline" className="text-2xs">
              {realmsLabel(config.realms)}
            </Badge>
          </div>
          <p className="text-xs text-muted-foreground">
            {runtime.note_count} notes
            {runtime.last_export_at ? ` · exported ${relTime(runtime.last_export_at)}` : ""}
            {runtime.last_commit ? ` · head ${runtime.last_commit.slice(0, 8)}` : ""}
            {config.export_interval_secs > 0
              ? ` · every ${Math.round(config.export_interval_secs / 60)}m`
              : " · manual exports"}
          </p>
        </div>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={() => exportNow.mutate()}
            disabled={exportNow.isPending}
          >
            <RefreshCw className="mr-1 h-3.5 w-3.5" />
            {exportNow.isPending ? "Exporting…" : "Export now"}
          </Button>
          {config.gardener.enabled && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => garden.mutate()}
              disabled={garden.isPending}
            >
              <Flower2 className="mr-1 h-3.5 w-3.5" /> Garden
            </Button>
          )}
        </div>
      </div>

      <CloneInstructions name={config.name} />

      <section>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Recent runs
        </h3>
        {runtime.runs.length === 0 ? (
          <p className="text-sm text-muted-foreground">No runs yet.</p>
        ) : (
          <div className="flex flex-col gap-1.5">
            {runtime.runs.slice(0, 20).map((run, i) => (
              <div
                key={`${run.started_at}-${i}`}
                className="flex items-center gap-3 rounded-md border bg-background px-3 py-2 text-xs"
              >
                <StatusPill tone={runTone(run)}>
                  {run.error ? "error" : run.noop ? "noop" : "ok"}
                </StatusPill>
                <span className="w-24 shrink-0 font-medium">{run.kind}</span>
                <span className="w-20 shrink-0 text-muted-foreground">
                  {relTime(run.started_at)}
                </span>
                <span className="truncate text-muted-foreground">{runDetail(run)}</span>
              </div>
            ))}
          </div>
        )}
      </section>

      <section className="rounded-lg border border-destructive/30 p-3">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="text-sm font-medium">Delete this brain</p>
            <p className="text-xs text-muted-foreground">
              Removes the published repo and registry entry. Memories are NOT deleted.
            </p>
          </div>
          {confirming ? (
            <div className="flex gap-2">
              <Button size="sm" variant="ghost" onClick={() => setConfirming(false)}>
                Cancel
              </Button>
              <Button
                size="sm"
                variant="destructive"
                disabled={del.isPending}
                onClick={() =>
                  del.mutate(config.name, {
                    onSuccess: () => void navigate({ to: "/brains" }),
                  })
                }
              >
                Confirm delete
              </Button>
            </div>
          ) : (
            <Button size="sm" variant="outline" onClick={() => setConfirming(true)}>
              <Trash2 className="mr-1 h-3.5 w-3.5" /> Delete
            </Button>
          )}
        </div>
      </section>
    </div>
  );
}
