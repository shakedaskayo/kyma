// Shim: re-exports discover-live types and LiveSession from @pensieve-ai/client.
// LiveSession takes { endpoint, token } directly (not a transport) — re-exported as-is.
export { LiveSession, type LiveStatus, type LiveBody } from "@pensieve-ai/client";
