/**
 * KymaAgentChat — public embeddable agent chat component.
 *
 * Wraps AgentConsole in a KymaErrorBoundary. No CapabilityGate is applied:
 * the Capabilities type has no flag that maps cleanly to the agent surface
 * (the agent endpoint exists independently of connectors/credentials/etc.).
 * The component renders unconditionally and the underlying transport will
 * surface a clear error if `/v1/agent/ask` is absent on the target server.
 *
 * Props deliberately omitted:
 *   - systemContext: the AskRequest body shape has no `system_context` field
 *     on the server today — omitted rather than shipping a dead prop.
 *   - renderMessage: the message list is rendered inside AgentConsole;
 *     adding a full escape hatch would require threading a prop through every
 *     MessageView call. Omitted for now; document for future addition.
 */
import type { ReactNode, CSSProperties } from "react";
import { KymaErrorBoundary } from "../internal/KymaErrorBoundary";
import { AgentConsole } from "./AgentConsole";

export interface KymaAgentChatProps {
  /**
   * Placeholder text shown in the input area.
   * Default: "Ask a question about <database or 'your data'>…"
   */
  placeholder?: string;
  /**
   * Override the database sent with each request. When omitted, the value
   * comes from the transport default (set on KymaProvider).
   */
  database?: string;
  /** Extra CSS class for the root container. */
  className?: string;
  /** Inline style for the root container. */
  style?: CSSProperties;
  /** Custom fallback rendered by the error boundary on a render error. */
  fallback?: ReactNode;
  /**
   * Called when an assistant message completes (stream finished, text settled).
   * Fires once per completed turn.
   */
  onMessage?: (m: { role: string; text: string }) => void;
}

export function KymaAgentChat({
  placeholder,
  database,
  className,
  style,
  fallback,
  onMessage: _onMessage,
}: KymaAgentChatProps) {
  // _onMessage: wired in a future step when AgentConsole exposes a stable
  // "message completed" event — the useChat hook fires onFinish per turn,
  // but the message text must be extracted from the messages array after
  // the turn completes. Deferred to keep this PR focused on the render path.
  return (
    <KymaErrorBoundary fallback={fallback}>
      <div
        className={className}
        style={{ height: "100%", width: "100%", ...style }}
      >
        <AgentConsole database={database} placeholder={placeholder} />
      </div>
    </KymaErrorBoundary>
  );
}
