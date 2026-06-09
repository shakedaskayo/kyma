/**
 * KymaAgentChat — public embeddable agent chat component.
 *
 * Wraps AgentConsole in a KymaErrorBoundary + CapabilityGate on the `agent`
 * capability flag: servers without the inline agent surface render a
 * "not available on this server" card instead of 404-ing (the gate fails
 * OPEN while capabilities load — servers predating the flag report it via
 * FULL_CAPABILITIES defaulting).
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
import { CapabilityGate } from "../internal/CapabilityGate";
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
  onMessage,
}: KymaAgentChatProps) {
  return (
    <KymaErrorBoundary fallback={fallback}>
      <div
        className={className}
        style={{ height: "100%", width: "100%", ...style }}
      >
        <CapabilityGate capability="agent">
          <AgentConsole
            database={database}
            placeholder={placeholder}
            onAssistantMessage={onMessage}
          />
        </CapabilityGate>
      </div>
    </KymaErrorBoundary>
  );
}
