import { Component, type ErrorInfo, type ReactNode } from "react";

/** Without this, one rendering error leaves an empty window until Riptor is reopened. */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div role="alert" className="flex h-screen flex-col items-center justify-center gap-3 bg-bg p-6 text-center text-fg">
        <h1 className="text-lg font-semibold">Riptor's window hit a problem</h1>
        <p className="text-sm text-muted">Your torrents keep running. Reload the window to get back to them.</p>
        <p className="max-w-xl break-words font-mono text-xs text-muted">{error.message}</p>
        <button className="btn btn-primary" onClick={() => window.location.reload()}>
          Reload window
        </button>
      </div>
    );
  }
}
