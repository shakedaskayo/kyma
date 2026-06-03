import { KeyRound, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonRows } from "@/components/ui/skeleton";
import { credentialKindLabel } from "@/sdk/credentials";
import { useCredentials, useDeleteCredential } from "./useCredentials";
import { CredentialIcon } from "./CredentialIcon";

function relTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  if (Number.isNaN(diff)) return "—";
  const s = Math.round(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

export function CredentialsList({ onAdd }: { onAdd: () => void }) {
  const { data, isLoading, error } = useCredentials();
  const del = useDeleteCredential();

  if (isLoading) {
    return <SkeletonRows rows={6} className="mx-auto max-w-3xl py-2" />;
  }
  if (error) {
    return (
      <div className="flex items-center justify-center py-20 text-sm text-destructive">
        Failed to load credentials: {(error as Error).message}
      </div>
    );
  }
  if (!data || data.length === 0) {
    return (
      <EmptyState
        icon={KeyRound}
        title="No credentials yet"
        description="Store typed secrets once — PATs, basic auth, connection URLs, AWS keys, OAuth tokens — and reference them from connectors and other subsystems by id. Encrypted at rest."
        action={<Button onClick={onAdd}>Add credential</Button>}
      />
    );
  }
  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-2">
      {data.map((c) => (
        <div
          key={c.id}
          className="group flex items-center gap-4 rounded-lg border bg-background px-4 py-3 shadow-sm transition hover:border-primary/40 hover:shadow-md"
        >
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border bg-background text-muted-foreground">
            <CredentialIcon kind={c.kind} size={18} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate font-medium text-foreground">{c.label}</span>
              <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
                {credentialKindLabel(c.kind)}
              </span>
            </div>
            <div className="mt-0.5 flex items-center gap-3 text-xs text-muted-foreground">
              <span className="font-mono tabular-nums">{c.preview}</span>
              <span>Added {relTime(c.created_at)}</span>
            </div>
          </div>
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8 text-muted-foreground opacity-0 transition group-hover:opacity-100 hover:text-destructive"
            title="Delete"
            disabled={del.isPending}
            onClick={() => {
              if (confirm(`Delete credential "${c.label}"? Anything using it will lose access.`)) {
                del.mutate(c.id);
              }
            }}
          >
            <Trash2 />
          </Button>
        </div>
      ))}
    </div>
  );
}
