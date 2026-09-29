import { invoke } from '@tauri-apps/api/core'

type ReportDetails = Record<string, unknown> | string | undefined

function serializeDetails(details: ReportDetails): string {
  if (details == null) return ''
  if (typeof details === 'string') return details
  try {
    return JSON.stringify(details, null, 2)
  } catch {
    return String(details)
  }
}

/** Błędy UI / JS → PocketBase przez Rust (token nie trafia do bundla frontu). */
export function reportError(message: string, details?: ReportDetails): void {
  const msg = (message || 'Unknown error').slice(0, 500)
  const det = serializeDetails(details).slice(0, 12_000)
  invoke('report_frontend_error', { message: msg, details: det }).catch(() => {
    /* ignore — raportowanie nie może psuć UX */
  })
}

export function installGlobalErrorHandlers(): void {
  window.addEventListener('error', (event) => {
    const err = event.error
    const message = err?.message || event.message || 'window.onerror'
    const stack = err?.stack || `${event.filename}:${event.lineno}:${event.colno}`
    reportError(message, { kind: 'window.error', stack })
  })

  window.addEventListener('unhandledrejection', (event) => {
    const reason = event.reason
    const message =
      reason instanceof Error
        ? reason.message
        : typeof reason === 'string'
          ? reason
          : 'unhandledrejection'
    const stack = reason instanceof Error ? reason.stack : String(reason)
    reportError(message, { kind: 'unhandledrejection', stack })
  })
}
