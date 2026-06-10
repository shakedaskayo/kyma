import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { LayoutDashboard, Plus } from "lucide-react";
import { motion } from "framer-motion";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonCard } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { staggerContainer, listItem, useMotionSafe } from "@/lib/motion";
import { useSession } from "@/sdk/session";
import { listDashboards, createDashboard } from "@/sdk/dashboards";

export const Route = createFileRoute("/_app/dashboards/")({
  component: DashboardsListPage,
});

function DashboardsListPage() {
  const { endpoint, token } = useSession();
  const navigate = useNavigate();
  const qc = useQueryClient();

  const [newOpen, setNewOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newDesc, setNewDesc] = useState("");

  const { data: dashboards, isLoading } = useQuery({
    queryKey: ["dashboards", endpoint],
    queryFn: () => listDashboards({ endpoint, token }),
    staleTime: 30_000,
    enabled: Boolean(endpoint),
  });

  const { mutate: create, isPending } = useMutation({
    mutationFn: () =>
      createDashboard({
        endpoint,
        token,
        body: { name: newName, description: newDesc || undefined },
      }),
    onSuccess: (d) => {
      void qc.invalidateQueries({ queryKey: ["dashboards", endpoint] });
      setNewOpen(false);
      setNewName("");
      setNewDesc("");
      void navigate({ to: "/dashboards/$id", params: { id: d.id }, search: { edit: "1" as string | undefined } });
    },
  });

  const handleCreate = () => {
    if (!newName.trim()) return;
    create();
  };

  const stagger = useMotionSafe(staggerContainer);
  const item = useMotionSafe(listItem);

  function fmtDate(iso: string) {
    const d = new Date(iso);
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
  }

  return (
    <div className="flex h-full flex-col bg-muted/20">
      {/* Header */}
      <div className="flex items-center justify-between border-b bg-background px-6 py-4">
        <div>
          <h1 className="text-lg font-semibold">Dashboards</h1>
          <p className="text-sm text-muted-foreground">Compose queries and charts on a single page.</p>
        </div>
        <Button onClick={() => setNewOpen(true)}>
          <Plus className="mr-1.5 h-4 w-4" /> New dashboard
        </Button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6">
        {isLoading && (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            {Array.from({ length: 8 }).map((_, i) => (
              <SkeletonCard key={i} />
            ))}
          </div>
        )}

        {!isLoading && dashboards && dashboards.length === 0 && (
          <EmptyState
            icon={LayoutDashboard}
            title="No dashboards yet"
            description="Create one to compose multiple queries and charts on a single page."
            action={
              <Button onClick={() => setNewOpen(true)}>
                <Plus className="mr-1.5 h-4 w-4" /> Create dashboard
              </Button>
            }
          />
        )}

        {!isLoading && dashboards && dashboards.length > 0 && (
          <motion.div
            variants={stagger}
            initial="hidden"
            animate="show"
            className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4"
          >
            {dashboards.map((d) => (
              <motion.button
                key={d.id}
                variants={item}
                onClick={() => navigate({ to: "/dashboards/$id", params: { id: d.id }, search: { edit: undefined } })}
                className="group rounded-lg border bg-card p-4 text-left shadow-elev-2 transition-[transform,box-shadow,border-color] duration-150 hover:-translate-y-0.5 hover:border-border-strong hover:shadow-elev-3"
              >
                <div className="flex items-start justify-between gap-2">
                  <LayoutDashboard className="mt-0.5 h-4 w-4 shrink-0 text-primary/70 group-hover:text-primary" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-medium text-foreground">{d.name}</div>
                    {d.description && (
                      <div className="mt-0.5 truncate text-xs text-muted-foreground">
                        {d.description}
                      </div>
                    )}
                  </div>
                </div>
                <div className="mt-3 flex items-center justify-between text-[11px] text-muted-foreground">
                  <span>Updated {fmtDate(d.updated_at)}</span>
                  <span className="rounded-full bg-muted px-2 py-0.5">{d.time_range_preset}</span>
                </div>
              </motion.button>
            ))}
          </motion.div>
        )}
      </div>

      {/* New Dashboard dialog */}
      <Dialog open={newOpen} onOpenChange={(o) => !o && setNewOpen(false)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New dashboard</DialogTitle>
          </DialogHeader>
          <div className="space-y-4 py-1">
            <div className="space-y-1">
              <Label htmlFor="dash-name">Name *</Label>
              <Input
                id="dash-name"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="My Dashboard"
                onKeyDown={(e) => e.key === "Enter" && handleCreate()}
                autoFocus
              />
            </div>
            <div className="space-y-1">
              <Label htmlFor="dash-desc">Description</Label>
              <Input
                id="dash-desc"
                value={newDesc}
                onChange={(e) => setNewDesc(e.target.value)}
                placeholder="Optional description…"
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setNewOpen(false)}>
              Cancel
            </Button>
            <Button onClick={handleCreate} disabled={!newName.trim() || isPending}>
              {isPending ? "Creating…" : "Create"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
