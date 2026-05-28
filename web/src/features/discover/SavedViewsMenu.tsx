import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { createSavedView } from "@/sdk/discover";
import type { Scope } from "./types";

type Props = { currentScope: Scope; onSaved?: (id: string) => void };

export function SavedViewsMenu({ currentScope, onSaved }: Props) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const qc = useQueryClient();

  const canSave = currentScope.kind === "sources" && currentScope.sources.length > 0;

  const m = useMutation({
    mutationFn: async () => {
      if (currentScope.kind !== "sources") {
        throw new Error("only explicit source scopes can be saved as views in v1");
      }
      return createSavedView(name.trim(), currentScope.sources);
    },
    onSuccess: (v) => {
      qc.invalidateQueries({ queryKey: ["saved-views"] });
      setOpen(false);
      setName("");
      onSaved?.(v.id);
    },
  });

  return (
    <>
      <Button
        variant="outline"
        size="sm"
        disabled={!canSave}
        onClick={() => setOpen(true)}
        title={canSave ? "Save current scope as a named view" : "Pick db/tables first"}
      >
        Save as view…
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Save scope as view</DialogTitle>
          </DialogHeader>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. prod-logs"
            autoFocus
          />
          {m.error && (
            <div className="text-sm text-destructive">
              {(m.error as Error).message}
            </div>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setOpen(false)}
              disabled={m.isPending}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => m.mutate()}
              disabled={!name.trim() || m.isPending}
            >
              {m.isPending ? "Saving…" : "Save"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
