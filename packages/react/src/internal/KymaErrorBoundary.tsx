/**
 * KymaErrorBoundary — class-based error boundary that:
 *  - catches render errors in its subtree
 *  - renders a styled fallback card (or the `fallback` prop override)
 *  - exposes a Retry button that resets the boundary
 *  - reports to the context `onError` callback if one is set
 *
 * Uses a functional wrapper (KymaErrorBoundary) that reads the context and
 * passes `onError` into the class boundary via props.
 */
import { Component, type ReactNode } from "react";
import { useKymaContext } from "../provider/context";

// ── Class boundary (cannot use hooks) ────────────────────────────────────────

interface ClassBoundaryProps {
  children: ReactNode;
  fallback?: ReactNode;
  /** Called when an error is caught — wired from the Kyma context. */
  onError?: (err: unknown) => void;
}

interface ClassBoundaryState {
  hasError: boolean;
  error: unknown;
}

class ClassBoundary extends Component<ClassBoundaryProps, ClassBoundaryState> {
  constructor(props: ClassBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: unknown): ClassBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: unknown) {
    this.props.onError?.(error);
  }

  handleRetry = () => {
    this.setState({ hasError: false, error: null });
  };

  override render() {
    if (!this.state.hasError) return this.props.children;

    // Custom fallback prop takes full control
    if (this.props.fallback != null) return this.props.fallback;

    // Default fallback: styled inline error card with Retry button
    const message =
      this.state.error instanceof Error
        ? this.state.error.message
        : String(this.state.error ?? "An unexpected error occurred");

    return (
      <div className="ky-rounded-lg ky-border ky-border-border ky-bg-card ky-p-4 ky-text-sm ky-text-foreground">
        <p className="ky-font-medium ky-text-destructive ky-mb-2">Something went wrong</p>
        <p className="ky-text-muted-foreground ky-mb-3">{message}</p>
        <button
          onClick={this.handleRetry}
          className="ky-rounded-md ky-border ky-border-border ky-bg-background ky-px-3 ky-py-1.5 ky-text-sm ky-text-foreground hover:ky-bg-accent"
        >
          Retry
        </button>
      </div>
    );
  }
}

// ── Public functional wrapper (reads context via hook) ────────────────────────

export interface KymaErrorBoundaryProps {
  children: ReactNode;
  fallback?: ReactNode;
}

export function KymaErrorBoundary({ children, fallback }: KymaErrorBoundaryProps) {
  // Best-effort: if rendered outside a provider (e.g. in tests without one),
  // skip the context read and pass no onError — the boundary still works.
  let onError: ((err: unknown) => void) | undefined;
  try {
    // eslint-disable-next-line react-hooks/rules-of-hooks
    const ctx = useKymaContext();
    onError = ctx.onError;
  } catch {
    // Outside provider — acceptable; boundary works without onError
  }

  return (
    <ClassBoundary onError={onError} fallback={fallback}>
      {children}
    </ClassBoundary>
  );
}
