import { createFileRoute, useNavigate, Link } from "@tanstack/react-router";
import { ArrowLeft, Share2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { toast } from "sonner";
import { KymaDashboard } from "@kyma-ai/react";
import type { DashboardWithPanels } from "@kyma-ai/react";

export const Route = createFileRoute("/_app/dashboards/$id")({
  validateSearch: (s: Record<string, unknown>) => ({
    edit: typeof s.edit === "string" ? s.edit : undefined,
  }),
  component: DashboardPage,
});

function DashboardPage() {
  const { id } = Route.useParams();
  const search = Route.useSearch();
  const navigate = useNavigate();

  const editable = search.edit === "1";

  const handleEditChange = (next: boolean) => {
    void navigate({
      to: "/dashboards/$id",
      params: { id },
      search: { edit: next ? "1" : "0" },
      replace: true,
    });
  };

  const handleSaveSuccess = (_dashboard: DashboardWithPanels) => {
    toast.success("Dashboard saved.");
  };

  const handleSaveError = (err: unknown) => {
    toast.error(`Save failed: ${(err as Error).message}`);
  };

  const handleShare = () => {
    const url = new URL(window.location.href);
    url.searchParams.set("edit", "0");
    void navigator.clipboard.writeText(url.toString()).then(() => {
      toast.success("Link copied — anyone with access can view this dashboard.");
    });
  };

  return (
    <div className="flex h-full flex-col">
      {/* ── Web chrome: back + share ──────────────────────────────────────── */}
      <div className="flex shrink-0 items-center gap-2 border-b bg-background px-4 py-2">
        <Button variant="ghost" size="sm" asChild>
          <Link to="/dashboards">
            <ArrowLeft className="h-4 w-4" />
          </Link>
        </Button>
        <div className="ml-auto">
          <Button size="sm" variant="outline" onClick={handleShare}>
            <Share2 className="mr-1 h-3.5 w-3.5" /> Share
          </Button>
        </div>
      </div>

      {/* ── Package dashboard (toolbar + grid) ───────────────────────────── */}
      <KymaDashboard
        dashboardId={id}
        editable={editable}
        onEditChange={handleEditChange}
        onSaveSuccess={handleSaveSuccess}
        onSaveError={handleSaveError}
        className="flex-1"
      />
    </div>
  );
}
