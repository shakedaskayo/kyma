import { Card, CardContent } from "@/components/ui/card";

export function AppearanceSettings() {
  return (
    <Card>
      <CardContent className="space-y-2 p-4 text-sm text-muted-foreground">
        <p>
          Theme controls live in the header (top-right of the app) — click the
          sun / moon / monitor icons to switch between light, dark, and system.
        </p>
        <p className="text-xs">
          A richer appearance panel (font size, density, etc.) will land here in
          a future release.
        </p>
      </CardContent>
    </Card>
  );
}
