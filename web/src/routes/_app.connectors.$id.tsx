import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useConnector } from "@/features/connectors/useConnectors";
import { ConnectorDetail } from "@/features/connectors/ConnectorDetail";

export const Route = createFileRoute("/_app/connectors/$id")({
  component: ConnectorDetailPage,
});

function ConnectorDetailPage() {
  const { id } = Route.useParams();
  const navigate = useNavigate();
  const { data: detail, isLoading, error } = useConnector(id);

  return (
    <div className="flex h-full flex-col bg-muted/10">
      {/* Toolbar */}
      <div className="flex items-center gap-2 border-b bg-background px-4 py-2 shadow-sm">
        <Button variant="ghost" size="sm" asChild>
          <Link to="/connectors">
            <ArrowLeft className="h-4 w-4" />
          </Link>
        </Button>
        <span className="text-sm font-semibold">Connector</span>
      </div>

      <div className="flex-1 overflow-y-auto p-6">
        {isLoading && (
          <div className="flex items-center justify-center py-20 text-sm text-muted-foreground animate-pulse">
            Loading connector…
          </div>
        )}
        {error && (
          <div className="flex flex-col items-center justify-center py-20 text-center">
            <p className="text-sm text-muted-foreground">
              {(error as Error).message === "not found"
                ? "Connector not found."
                : `Failed to load: ${(error as Error).message}`}
            </p>
            <Button className="mt-4" variant="outline" asChild>
              <Link to="/connectors">Back to connectors</Link>
            </Button>
          </div>
        )}
        {detail && (
          <ConnectorDetail detail={detail} onDeleted={() => navigate({ to: "/connectors" })} />
        )}
      </div>
    </div>
  );
}
