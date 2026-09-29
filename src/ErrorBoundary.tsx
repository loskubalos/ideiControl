import { Component, type ErrorInfo, type ReactNode } from 'react'
import { reportError } from './errorReporting'

type Props = { children: ReactNode }
type State = { hasError: boolean }

/** Łapie nieobsłużone wyjątki React i wysyła je do PocketBase. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false }

  static getDerivedStateFromError(): State {
    return { hasError: true }
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    reportError(error.message || 'React render error', {
      kind: 'react',
      stack: error.stack,
      componentStack: info.componentStack,
    })
  }

  render() {
    if (this.state.hasError) {
      return (
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100%',
            gap: 12,
            padding: 24,
            color: 'var(--c-text, #e8e8e8)',
            background: 'var(--c-bg, #121212)',
            fontFamily: 'system-ui, sans-serif',
          }}
        >
          <div style={{ fontSize: 16 }}>Something went wrong</div>
          <div style={{ fontSize: 13, color: 'var(--c-text-muted, #999)', textAlign: 'center' }}>
            The error was reported. Try reloading the app.
          </div>
          <button
            type="button"
            onClick={() => this.setState({ hasError: false })}
            style={{
              marginTop: 8,
              padding: '8px 16px',
              borderRadius: 8,
              border: '0.5px solid var(--c-border, #333)',
              background: 'var(--c-seg, #222)',
              color: 'inherit',
              cursor: 'pointer',
            }}
          >
            Try again
          </button>
        </div>
      )
    }
    return this.props.children
  }
}
