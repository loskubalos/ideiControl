export type Lang = 'pl' | 'en'

export type MediaKeyAction =
  | 'none'
  | 'play_pause'
  | 'next_track'
  | 'previous_track'
  | 'stop'
  | 'mute'
  | 'volume_up'
  | 'volume_down'

export type ButtonBinding =
  | { kind: 'none' }
  | { kind: 'media'; action: MediaKeyAction }
  | { kind: 'shortcut'; vk: number; mods: number; label: string }

export type VolumeTarget =
  | { type: 'system' }
  | { type: 'mic' }
  | { type: 'app'; pid: number; name?: string }
  | { type: 'category'; id: string }

export interface AudioSessionInfo {
  pid: number
  name: string
  exe_name?: string
  display_name?: string
  is_active?: boolean
  is_playing?: boolean
  is_game?: boolean
}

export interface DeviceInfo {
  port: string
  model: string
  sliders: number
  proto?: number
  fw?: string
  buttons?: number
  caps?: string[]
}

export type ShortcutLedMode = 'off' | 'momentary' | 'toggle'

/** Snapshot danych profilu (snake_case — zgodne z Rust / Tauri). */
export type ProfileData = {
  volume_assignments: VolumeTarget[][]
  button_bindings: ButtonBinding[]
  button_hw_slider_mute: boolean[]
  shortcut_led_mode: ShortcutLedMode[]
}

export type Profile = ProfileData & {
  id: string
  name: string
  is_default?: boolean
}

export type ProfilesState = {
  profiles: Profile[]
  active_profile_id: string
}

export type AppUpdateInfo = {
  available: boolean
  current_version: string
  latest_version?: string
  body?: string
}

export type UpdateDownloadProgress = {
  downloaded: number
  total?: number
  percent?: number
  phase: string
}

/** @deprecated Używaj Profile — zachowane dla migracji localStorage. */
export type PresetEntry = {
  id: string
  name: string
  sliderValues: number[]
  assignments: VolumeTarget[][]
  buttonBindings?: ButtonBinding[]
  buttonMediaActions?: MediaKeyAction[]
  buttonHwSliderMute?: boolean[]
  shortcutLedMode?: ShortcutLedMode[]
  shortcutMuteLedMap?: boolean[]
  shortcutMuteLed?: boolean
}
