import { invoke } from '@tauri-apps/api/core'

type TelemetryPayload = Record<string, unknown>

/** Wysyłka tylko przez Tauri (Rust) — z Bearer tokenem na Twoim proxy; bez tego publiczny Umami przyjmuje śmieci z całego internetu. */
export function trackTelemetryEvent(name: string, data: TelemetryPayload = {}): void {
  invoke('send_telemetry', { eventName: name, payload: data }).catch(() => {
    /* ignore */
  })
}

export function trackAppStartedOnce(): void {
  const key = 'telemetry-app-started-sent'
  if (sessionStorage.getItem(key) === '1') return
  sessionStorage.setItem(key, '1')
  trackTelemetryEvent('app_started')
}
