import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useDataSource } from "@/features/datasources/useDataSources";
import { DataSourceDetail } from "@/features/datasources/DataSourceDetail";

// Detail page for a single data source. Also the landing spot for the OAuth
// callback's fallback redirect (/data-sources/new?provider=…&oauth=…): the
// query params aren't consumed here — the real result rides postMessage /
// flow polling in useOAuthConnect — so id="new" just renders the not-found
// state with a way back, exactly as the old route did.
export const Route = createFileRoute("/_app/data-sources/$id")({
  component: DataSourceDetailPage,
});

function DataSourceDetailPage() {
  const { id } = Route.useParams();
  const navigate = useNavigate();
  const { data: detail, isLoading, error } = useDataSource(id);

  return (
    <div className="flex h-full flex-col bg-muted/10">
      {/* Toolbar */}
      <div className="flex items-center gap-2 border-b bg-background px-4 py-2 shadow-sm">
        <Button variant="ghost" size="sm" asChild>
          <Link to="/data-sources">
            <ArrowLeft className="h-4 w-4" />
          </Link>
        </Button>
        <span className="text-sm font-semibold">Data source</span>
      </div>

      <div className="flex-1 overflow-y-auto p-6">
        {isLoading && (
          <div className="flex items-center justify-center py-20 text-sm text-muted-foreground animate-pulse">
            Loading data source…
          </div>
        )}
        {error && (
          <div className="flex flex-col items-center justify-center py-20 text-center">
            <p className="text-sm text-muted-foreground">
              {(error as Error).message === "not found"
                ? "Data source not found."
                : `Failed to load: ${(error as Error).message}`}
            </p>
            <Button className="mt-4" variant="outline" asChild>
              <Link to="/data-sources">Back to data sources</Link>
            </Button>
          </div>
        )}
        {detail && (
          <DataSourceDetail detail={detail} onDeleted={() => navigate({ to: "/data-sources" })} />
        )}
      </div>
    </div>
  );
}
