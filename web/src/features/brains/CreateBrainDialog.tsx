import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useCreateBrain } from "./useBrains";

const SCHEDULES: Array<{ label: string; secs: number }> = [
  { label: "15 min", secs: 900 },
  { label: "hourly", secs: 3600 },
  { label: "daily", secs: 86400 },
  { label: "manual", secs: 0 },
];

export function CreateBrainDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const [realms, setRealms] = useState("");
  const [allRealms, setAllRealms] = useState(false);
  const [schedule, setSchedule] = useState(900);
  const [gardener, setGardener] = useState(false);
  const create = useCreateBrain();

  const validName = /^[a-z0-9][a-z0-9_-]{0,63}$/.test(name);
  const realmList = realms
    .split(",")
    .map((r) => r.trim())
    .filter(Boolean);
  const canSubmit = validName && (allRealms || realmList.length > 0) && !create.isPending;

  const submit = () => {
    create.mutate(
      {
        name,
        realms: allRealms ? [] : realmList,
        all_realms: allRealms,
        export_interval_secs: schedule,
        gardener: gardener ? { enabled: true } : undefined,
      },
      { onSuccess: () => onOpenChange(false) },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Publish a brain</DialogTitle>
          <DialogDescription>
            A brain is a Git repository rendered from memory as an Obsidian vault — clone it,
            grep it, push edits back. It re-exports on the schedule below.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4 py-1">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="brain-name">Name</Label>
            <Input
              id="brain-name"
              placeholder="team"
              value={name}
              onChange={(e) => setName(e.target.value.toLowerCase())}
            />
            {name && !validName && (
              <p className="text-2xs text-destructive">
                lowercase letters, digits, `-`/`_`, up to 64 chars
              </p>
            )}
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="brain-realms">Realms</Label>
            <Input
              id="brain-realms"
              placeholder="pensieve, global"
              value={realms}
              disabled={allRealms}
              onChange={(e) => setRealms(e.target.value)}
            />
            <label className="flex items-center gap-2 text-xs text-muted-foreground">
              <input
                type="checkbox"
                checked={allRealms}
                onChange={(e) => setAllRealms(e.target.checked)}
              />
              include every realm
            </label>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>Export schedule</Label>
            <div className="flex gap-1.5">
              {SCHEDULES.map((s) => (
                <Button
                  key={s.label}
                  type="button"
                  size="sm"
                  variant={schedule === s.secs ? "default" : "outline"}
                  className="h-7 px-2.5 text-xs"
                  onClick={() => setSchedule(s.secs)}
                >
                  {s.label}
                </Button>
              ))}
            </div>
          </div>
          <label className="flex items-start gap-2 text-xs text-muted-foreground">
            <input
              type="checkbox"
              className="mt-0.5"
              checked={gardener}
              onChange={(e) => setGardener(e.target.checked)}
            />
            <span>
              <span className="font-medium text-foreground">Wiki gardener</span> — an agent
              curates wiki/ overview pages and merges duplicates between exports (uses your
              configured agent engine).
            </span>
          </label>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={!canSubmit} onClick={submit}>
            {create.isPending ? "Publishing…" : "Publish brain"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
