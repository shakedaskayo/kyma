import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { BookOpenText, GitBranch, Plus } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonRows } from "@/components/ui/skeleton";
import { realmsLabel, type Brain } from "@/sdk/brains";
import { relTime } from "@/lib/time";
import { CreateBrainDialog } from "./CreateBrainDialog";
import { useBrains } from "./useBrains";

function BrainRow({ brain }: { brain: Brain }) {
  const { config, runtime } = brain;
  return (
    <Link
      to="/brains/$name"
      params={{ name: config.name }}
      className="group flex items-center gap-4 rounded-lg border bg-background px-4 py-3 transition-colors hover:border-primary/40 hover:bg-muted/40"
    >
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary ring-1 ring-inset ring-primary/15">
        <GitBranch className="h-4 w-4" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{config.name}</span>
          <Badge variant="outline" className="text-2xs">
            {realmsLabel(config.realms)}
          </Badge>
          {config.gardener.enabled && (
            <Badge variant="outline" className="text-2xs">
              gardener
            </Badge>
          )}
        </div>
        <p className="mt-0.5 truncate text-xs text-muted-foreground">
          {runtime.note_count} notes
          {runtime.last_export_at
            ? ` · exported ${relTime(runtime.last_export_at)}`
            : " · first export pending"}
          {runtime.last_commit ? ` · ${runtime.last_commit.slice(0, 8)}` : ""}
          {runtime.last_error ? " · last run failed" : ""}
        </p>
      </div>
      <code className="hidden shrink-0 rounded bg-muted px-2 py-1 font-mono text-2xs text-muted-foreground sm:block">
        /git/{config.name}.git
      </code>
    </Link>
  );
}

export function BrainsList() {
  const { data, isLoading, error } = useBrains();
  const [createOpen, setCreateOpen] = useState(false);

  if (isLoading) return <SkeletonRows rows={3} className="py-2" />;
  if (error) {
    return (
      <div className="flex items-center justify-center py-10 text-sm text-destructive">
        Failed to load brains: {(error as Error).message}
      </div>
    );
  }

  const brains = data?.brains ?? [];
  return (
    <div className="flex flex-col gap-3">
      {data && !data.git_available && (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">
          The server has no <code>git</code> binary — brains are read-only until it is installed.
        </div>
      )}
      {brains.length === 0 ? (
        <EmptyState
          icon={BookOpenText}
          title="No brains published yet"
          description="A brain is a Git-clonable Obsidian vault of this workspace's memory — clone it, grep it, open it in Obsidian, push edits back."
          action={
            <Button size="sm" onClick={() => setCreateOpen(true)} disabled={!data?.git_available}>
              <Plus className="mr-1 h-3.5 w-3.5" /> Publish a brain
            </Button>
          }
        />
      ) : (
        <>
          <div className="flex justify-end">
            <Button size="sm" onClick={() => setCreateOpen(true)} disabled={!data?.git_available}>
              <Plus className="mr-1 h-3.5 w-3.5" /> Publish brain
            </Button>
          </div>
          {brains.map((b) => (
            <BrainRow key={b.config.name} brain={b} />
          ))}
        </>
      )}
      <CreateBrainDialog open={createOpen} onOpenChange={setCreateOpen} />
    </div>
  );
}
