import { Component, type ErrorInfo, type ReactNode } from "react";

interface State {
  error: Error | null;
}

/** Keeps a rendering bug in one panel from blanking the whole window. */
export class ErrorBoundary extends Component<{ children: ReactNode; label?: string }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[PiLunch] ${this.props.label ?? "UI"} crashed:`, error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="empty-state selectable" data-testid="error-boundary" style={{ textAlign: "left", alignItems: "stretch" }}>
        <b style={{ color: "var(--err)" }}>{this.props.label ?? "This panel"} hit an error</b>
        <pre style={{ whiteSpace: "pre-wrap", fontSize: 12, margin: 0, maxHeight: 300, overflow: "auto" }}>
          {`${this.state.error.name}: ${this.state.error.message}\n\n${this.state.error.stack ?? ""}`}
        </pre>
        <button className="btn" onClick={() => this.setState({ error: null })}>
          Try again
        </button>
      </div>
    );
  }
}
