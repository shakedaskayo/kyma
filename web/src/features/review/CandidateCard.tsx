import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Check, Pencil, Undo2, X } from "lucide-react";
import { Badge, type BadgeTone } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { relTime } from "@/lib/time";
import type { ReviewAction, ReviewItem } from "@/sdk/review";
import type { MemoryOp } from "@kyma-ai/client";

const OP_LABEL: Record<MemoryOp, string> = {
  add: "Add",
  update: "Update",
  invalidate: "Invalidate",
  merge: "Merge",
  archive: "Archive",
  link_entity_cross_realm: "Cross-realm link",
  relationship_write: "Relationship",
  promote_file_candidate: "Promote candidate",
};

// Destructive ops read "hot"; additive/edit ops read calmer.
const OP_TONE: Record<MemoryOp, BadgeTone> = {
  add: "emerald",
  update: "cyan",
  invalidate: "rose",
  merge: "amber",
  archive: "amber",
  link_entity_cross_realm: "violet",
  relationship_write: "indigo",
  promote_file_candidate: "teal",
};

type Pair = { leftLabel: string; left: string; rightLabel?: string; right?: string };

function asStr(v: unknown): string {
  if (v == null) return "";
  return typeof v === "string" ? v : JSON.stringify(v);
}

/** A human-readable old⇄new view of an op payload. */
function describe(item: ReviewItem): Pair {
  const p = item.payload as Record<string, unknown>;
  switch (item.operation) {
    case "add":
      return { leftLabel: "New memory", left: asStr(p.content) };
    case "update":
      return {
        leftLabel: "Target",
        left: asStr(p.target_id),
        rightLabel: "New content",
        right: asStr(p.new_content),
      };
    case "invalidate": {
      const repl = (p.replacement as Record<string, unknown>) ?? {};
      return {
        leftLabel: "Invalidates",
        left: asStr(p.target_id),
        rightLabel: "Replacement",
        right: asStr(repl.content),
      };
    }
    case "merge":
      return {
        leftLabel: "Keep",
        left: asStr(p.into_id),
        rightLabel: "Fold in",
        right: (p.from_ids as string[] | undefined)?.join(", ") ?? "",
      };
    case "archive":
      return { leftLabel: "Archive", left: asStr(p.memory_id) };
    case "link_entity_cross_realm":
    case "relationship_write":
      return {
        leftLabel: "From",
        left: asStr(p.src),
        rightLabel: `→ ${asStr(p.rel)} →`,
        right: asStr(p.dst),
      };
    default:
      return { leftLabel: "Payload", left: asStr(p) };
  }
}

/** The single editable free-text field of a payload, if any (for edit-then-approve). */
function editablePath(item: ReviewItem): string[] | null {
  switch (item.operation) {
    case "update":
      return ["new_content"];
    case "invalidate":
      return ["replacement", "content"];
    case "add":
      return ["content"];
    default:
      return null;
  }
}

function readPath(obj: Record<string, unknown>, path: string[]): string {
  let cur: unknown = obj;
  for (const k of path) cur = (cur as Record<string, unknown> | undefined)?.[k];
  return asStr(cur);
}

function writePath(
  obj: Record<string, unknown>,
  path: string[],
  value: string,
): Record<string, unknown> {
  const clone = structuredClone(obj);
  let cur: Record<string, unknown> = clone;
  for (let i = 0; i < path.length - 1; i++) {
    cur[path[i]] = { ...(cur[path[i]] as Record<string, unknown>) };
    cur = cur[path[i]] as Record<string, unknown>;
  }
  cur[path[path.length - 1]] = value;
  return clone;
}

export interface CandidateCardProps {
  item: ReviewItem;
  selected?: boolean;
  onToggleSelect?: (id: string) => void;
  onAction: (action: ReviewAction, payload?: Record<string, unknown>) => void;
  busy?: boolean;
}

export function CandidateCard({
  item,
  selected,
  onToggleSelect,
  onAction,
  busy,
}: CandidateCardProps) {
  const pair = describe(item);
  const editPath = editablePath(item);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(() =>
    editPath ? readPath(item.payload, editPath) : "",
  );

  const isPending = item.status === "pending";
  const isPostHoc = item.status === "auto_applied";
  const conf = item.confidence;

  const approve = () => {
    const payload =
      editing && editPath ? writePath(item.payload, editPath, draft) : undefined;
    onAction("approve", payload);
  };

  return (
    <div
      className={cn(
        "rounded-xl border border-border/60 bg-card/40 p-3 shadow-elev-1 transition-colors",
        selected && "ring-1 ring-violet-400/50",
      )}
      data-testid="candidate-card"
    >
      <div className="flex items-center gap-2">
        {onToggleSelect && isPending && (
          <input
            type="checkbox"
            checked={Boolean(selected)}
            onChange={() => onToggleSelect(item.id)}
            aria-label="select candidate"
            className="accent-violet-500"
          />
        )}
        <Badge tone={OP_TONE[item.operation]}>{OP_LABEL[item.operation]}</Badge>
        {conf != null && (
          <span
            className={cn(
              "rounded-full px-2 py-0.5 text-2xs font-mono",
              conf < 0.6 ? "bg-rose-500/10 text-rose-500" : "bg-muted text-muted-foreground",
            )}
            title="model confidence"
          >
            conf {conf.toFixed(2)}
          </span>
        )}
        <span className="text-2xs text-muted-foreground">
          {item.source}
          {item.source_run_id && (
            <>
              {" · "}
              <Link
                to="/memory/dreaming/$runId"
                params={{ runId: item.source_run_id }}
                className="underline hover:text-foreground"
              >
                run
              </Link>
            </>
          )}
          {" · "}
          {relTime(item.created_at)}
        </span>
        <span className="ml-auto">
          <Badge variant="outline">{item.mode === "gate" ? "needs approval" : "applied"}</Badge>
        </span>
      </div>

      {item.reason && (
        <p className="mt-2 text-xs text-muted-foreground">{item.reason}</p>
      )}

      <div className="mt-2 grid gap-2 sm:grid-cols-2">
        <Field label={pair.leftLabel} value={pair.left} />
        {pair.rightLabel !== undefined &&
          (editing && editPath ? (
            <label className="block">
              <span className="text-2xs uppercase tracking-wide text-muted-foreground">
                {pair.rightLabel} (editing)
              </span>
              <textarea
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                rows={3}
                className="mt-1 w-full rounded border bg-background px-2 py-1 text-sm"
              />
            </label>
          ) : (
            <Field label={pair.rightLabel} value={pair.right ?? ""} />
          ))}
      </div>

      {(isPending || isPostHoc) && (
        <div className="mt-3 flex items-center gap-2">
          {isPending && (
            <>
              <Button size="xs" onClick={approve} disabled={busy}>
                <Check /> {editing ? "Save & approve" : "Approve"}
              </Button>
              <Button
                size="xs"
                variant="outline"
                onClick={() => onAction("reject")}
                disabled={busy}
              >
                <X /> Reject
              </Button>
              {editPath && (
                <Button
                  size="xs"
                  variant="ghost"
                  onClick={() => setEditing((v) => !v)}
                  disabled={busy}
                >
                  <Pencil /> {editing ? "Cancel edit" : "Edit"}
                </Button>
              )}
            </>
          )}
          {isPostHoc && (
            <Button
              size="xs"
              variant="outline"
              onClick={() => onAction("undo")}
              disabled={busy}
            >
              <Undo2 /> Undo
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-2xs uppercase tracking-wide text-muted-foreground">{label}</div>
      <div className="truncate text-sm" title={value}>
        {value || "—"}
      </div>
    </div>
  );
}
