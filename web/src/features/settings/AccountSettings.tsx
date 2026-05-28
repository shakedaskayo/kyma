import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { useSession } from "@/sdk/session";
import { logout } from "@/sdk/auth";

export function AccountSettings() {
  const session = useSession();
  const navigate = useNavigate();
  const [loggingOut, setLoggingOut] = useState(false);

  const handleLogout = async () => {
    setLoggingOut(true);
    try {
      if (session.token) {
        await logout({ endpoint: session.endpoint, token: session.token }).catch(() => {
          // best-effort
        });
      }
    } finally {
      session.reset();
      setLoggingOut(false);
      navigate({ to: "/login", search: { next: undefined } });
    }
  };

  if (!session.user) {
    return (
      <Card>
        <CardContent className="p-4 text-sm text-muted-foreground">
          Not signed in.
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardContent className="space-y-4 p-4">
        <div className="space-y-1">
          <p className="text-xs uppercase tracking-wider text-muted-foreground">
            Signed in as
          </p>
          <p className="text-sm">
            <span className="font-semibold">{session.user.username}</span>{" "}
            <span className="text-muted-foreground">({session.user.role})</span>
          </p>
        </div>
        <div className="space-y-1">
          <p className="text-xs uppercase tracking-wider text-muted-foreground">
            Server
          </p>
          <p className="font-mono text-xs">{session.endpoint || "—"}</p>
        </div>
        <div className="pt-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleLogout}
            disabled={loggingOut}
          >
            {loggingOut ? "Logging out…" : "Log out"}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
