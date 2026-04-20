import { useHealth } from "@/sdk/reconnect";

export function ReconnectBanner() {
  const online = useHealth((s) => s.online);
  if (online) return null;
  return (
    <div className="border-b border-destructive/40 bg-destructive/10 px-4 py-1 text-xs text-destructive">
      Reconnecting to kyma…
    </div>
  );
}
