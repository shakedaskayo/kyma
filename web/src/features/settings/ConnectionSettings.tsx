import { useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { toast } from "sonner";
import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent } from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { useSession } from "@/sdk/session";

export function ConnectionSettings() {
  const session = useSession();
  const { next } = useSearch({ from: "/settings" });
  const navigate = useNavigate();
  const [endpoint, setEndpoint] = useState(
    session.endpoint || "http://localhost:8080",
  );
  const [token, setToken] = useState(session.token);
  const [database, setDatabase] = useState(session.database || "obs");
  const [advancedOpen, setAdvancedOpen] = useState(false);

  function save() {
    const trimmedEndpoint = endpoint.trim().replace(/\/$/, "");
    if (!trimmedEndpoint) {
      toast.error("Server URL is required");
      return;
    }
    session.set({
      endpoint: trimmedEndpoint,
      token,
      database: database.trim() || "obs",
    });
    toast.success("Connection saved");
    if (next) {
      navigate({ to: next, search: {} as never });
    }
  }

  return (
    <Card>
      <CardContent className="space-y-4 p-4">
        <div className="space-y-1.5">
          <Label htmlFor="endpoint">Server URL</Label>
          <Input
            id="endpoint"
            value={endpoint}
            onChange={(e) => setEndpoint(e.target.value)}
            placeholder="http://localhost:8080"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="database">Default database</Label>
          <Input
            id="database"
            value={database}
            onChange={(e) => setDatabase(e.target.value)}
            placeholder="obs"
          />
        </div>

        <Collapsible open={advancedOpen} onOpenChange={setAdvancedOpen}>
          <CollapsibleTrigger asChild>
            <button
              type="button"
              className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
            >
              <ChevronDown
                className={`h-3.5 w-3.5 transition-transform ${advancedOpen ? "rotate-180" : ""}`}
              />
              API token (advanced)
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent className="space-y-1.5 pt-2">
            <Label htmlFor="token">
              Bearer token{" "}
              <span className="font-normal text-muted-foreground">
                (optional)
              </span>
            </Label>
            <Input
              id="token"
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="paste token from PENSIEVE_AUTH_TOKENS"
            />
            <p className="text-xs text-muted-foreground">
              Configure with{" "}
              <code className="font-mono">PENSIEVE_AUTH_TOKENS=token:read</code> on
              the server; leave blank and set{" "}
              <code className="font-mono">PENSIEVE_AUTH_DISABLED=1</code> for
              unauthenticated dev.
            </p>
          </CollapsibleContent>
        </Collapsible>

        <div className="flex gap-2 pt-2">
          <Button size="sm" onClick={save}>
            Save + connect
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => session.reset()}
          >
            Reset
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
