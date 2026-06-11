import { useEffect, useState, useCallback, useRef, useMemo } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { isEnabled, enable, disable } from '@tauri-apps/plugin-autostart'
import { TitleBar } from './TitleBar'
import { IconSliders, IconSettings, IconRefresh, IconX, IconCpu } from './icons'
import { V0ControllerCard } from './V0ControllerCard'
import { trackAppStartedOnce, trackTelemetryEvent } from './telemetry'
import './index.css'

interface DeviceInfo {
  port: string
  model: string
  sliders: number
  proto?: number
  fw?: string
  buttons?: number
  caps?: string[]
}

export type VolumeTarget =
  | { type: 'system' }
  | { type: 'mic' }
  | { type: 'app'; pid: number; name?: string }
  | { type: 'category'; id: string }

interface AudioSessionInfo {
  pid: number
  name: string
  is_active?: boolean
  is_game?: boolean
}

/** Maksymalna liczba suwaków w stanie / presetach — zgodna z `crate::audio::MAX_SLIDERS`. */
const MAX_SLIDERS = 5

/** Zgodne z `media_keys::MEDIA_BUTTON_SLOTS` i serde `snake_case`. */
const MEDIA_BUTTON_SLOTS = 5

type Lang = 'pl' | 'en'
const LANG_STORAGE_KEY = 'idei-lang'

export type MediaKeyAction =
  | 'none'
  | 'play_pause'
  | 'next_track'
  | 'previous_track'
  | 'stop'
  | 'mute'
  | 'volume_up'
  | 'volume_down'

const MEDIA_ACTION_OPTIONS: { value: MediaKeyAction; label: string }[] = [
  { value: 'none', label: 'None (no key)' },
  { value: 'play_pause', label: 'Play / pause' },
  { value: 'next_track', label: 'Next track' },
  { value: 'previous_track', label: 'Previous track' },
  { value: 'stop', label: 'Stop' },
  { value: 'mute', label: 'Mute (system)' },
  { value: 'volume_up', label: 'Volume up' },
  { value: 'volume_down', label: 'Volume down' },
]

/** Zgodne z `media_keys::ButtonBinding` (serde tag = kind). */
export type ButtonBinding =
  | { kind: 'none' }
  | { kind: 'media'; action: MediaKeyAction }
  | { kind: 'shortcut'; vk: number; mods: number; label: string }

const MOD_CTRL = 1
const MOD_SHIFT = 2
const MOD_ALT = 4
const MOD_WIN = 8

function defaultButtonBindings(): ButtonBinding[] {
  // Domyślnie: wszystkie przyciski wyłączone (None).
  return Array.from({ length: MEDIA_BUTTON_SLOTS }, () => ({ kind: 'none' } as ButtonBinding))
}

function mediaRowToBinding(action: MediaKeyAction): ButtonBinding {
  if (action === 'none') return { kind: 'none' }
  return { kind: 'media', action }
}

function parseButtonBinding(x: unknown): ButtonBinding | null {
  if (!x || typeof x !== 'object') return null
  const o = x as Record<string, unknown>
  if (o.kind === 'none') return { kind: 'none' }
  if (o.kind === 'media' && typeof o.action === 'string') {
    const a = o.action as MediaKeyAction
    if (MEDIA_ACTION_OPTIONS.some((opt) => opt.value === a)) {
      return a === 'none' ? { kind: 'none' } : { kind: 'media', action: a }
    }
  }
  if (
    o.kind === 'shortcut' &&
    typeof o.vk === 'number' &&
    typeof o.mods === 'number' &&
    typeof o.label === 'string'
  ) {
    return { kind: 'shortcut', vk: o.vk, mods: o.mods, label: o.label }
  }
  return null
}

/** Mapowanie `KeyboardEvent.code` → Windows VK (US layout). */
function keyboardCodeToVk(code: string): number | null {
  const map: Record<string, number> = {
    Space: 0x20,
    Enter: 0x0d,
    Tab: 0x09,
    Escape: 0x1b,
    Backspace: 0x08,
    Delete: 0x2e,
    Insert: 0x2d,
    Home: 0x24,
    End: 0x23,
    PageUp: 0x21,
    PageDown: 0x22,
    ArrowLeft: 0x25,
    ArrowUp: 0x26,
    ArrowRight: 0x27,
    ArrowDown: 0x28,
    Minus: 0xbd,
    Equal: 0xbb,
    BracketLeft: 0xdb,
    BracketRight: 0xdd,
    Backslash: 0xdc,
    Semicolon: 0xba,
    Quote: 0xde,
    Comma: 0xbc,
    Period: 0xbe,
    Slash: 0xbf,
    Backquote: 0xc0,
    NumpadDecimal: 0x6e,
    NumpadAdd: 0x6b,
    NumpadSubtract: 0x6d,
    NumpadMultiply: 0x6a,
    NumpadDivide: 0x6f,
    NumpadEnter: 0x0d,
  }
  for (let i = 0; i <= 9; i++) {
    map[`Digit${i}`] = 0x30 + i
  }
  for (let i = 1; i <= 12; i++) {
    map[`F${i}`] = 0x6f + i
  }
  for (let c = 65; c <= 90; c++) {
    map[`Key${String.fromCharCode(c)}`] = c
  }
  for (let i = 0; i <= 9; i++) {
    map[`Numpad${i}`] = 0x60 + i
  }
  return map[code] ?? null
}

function modifierMask(e: React.KeyboardEvent): number {
  let m = 0
  if (e.ctrlKey) m |= MOD_CTRL
  if (e.shiftKey) m |= MOD_SHIFT
  if (e.altKey) m |= MOD_ALT
  if (e.metaKey) m |= MOD_WIN
  return m
}

function isModifierCode(code: string): boolean {
  return (
    code === 'ControlLeft' ||
    code === 'ControlRight' ||
    code === 'ShiftLeft' ||
    code === 'ShiftRight' ||
    code === 'AltLeft' ||
    code === 'AltRight' ||
    code === 'MetaLeft' ||
    code === 'MetaRight'
  )
}

function shortcutAllowedWithoutModifier(code: string): boolean {
  if (/^F([1-9]|1[0-2])$/.test(code)) return true
  return (
    code === 'Escape' ||
    code === 'Tab' ||
    code === 'Delete' ||
    code === 'Insert' ||
    code === 'Home' ||
    code === 'End' ||
    code === 'PageUp' ||
    code === 'PageDown' ||
    code.startsWith('Arrow')
  )
}

function shortcutLabelFromEvent(e: React.KeyboardEvent): string {
  const parts: string[] = []
  if (e.ctrlKey) parts.push('Ctrl')
  if (e.shiftKey) parts.push('Shift')
  if (e.altKey) parts.push('Alt')
  if (e.metaKey) parts.push('Win')
  let main = e.key.length === 1 ? e.key.toUpperCase() : e.code
  if (main.startsWith('Key')) main = main.slice(3)
  if (main.startsWith('Digit')) main = main.slice(5)
  if (main === ' ') main = 'Space'
  parts.push(main)
  return parts.join('+')
}

function defaultHwSliderMute(): boolean[] {
  return Array.from({ length: MEDIA_BUTTON_SLOTS }, () => false)
}

function defaultShortcutMuteLedMap(): boolean[] {
  return Array.from({ length: MEDIA_BUTTON_SLOTS }, () => false)
}

function emptySliderValues(): number[] {
  return Array.from({ length: MAX_SLIDERS }, () => 0)
}

function emptyAssignments(): VolumeTarget[][] {
  return Array.from({ length: MAX_SLIDERS }, () => [] as VolumeTarget[])
}

type PresetEntry = {
  id: string
  name: string
  sliderValues: number[]
  assignments: VolumeTarget[][]
  buttonBindings?: ButtonBinding[]
  /** Stare presety — migrate → buttonBindings. */
  buttonMediaActions?: MediaKeyAction[]
  buttonHwSliderMute?: boolean[]
  /** Per suwak: Neo przy skrócie — SET_SHORTCUT_MUTE_LED_MAP. */
  shortcutMuteLedMap?: boolean[]
  /** Stary preset globalny — migrowane do mapy. */
  shortcutMuteLed?: boolean
}

/** Klucz presetów w storage — rozróżnia modele (np. 3 vs 5 suwaków, różną liczbę przycisków). */
function devicePresetKey(d: DeviceInfo): string {
  const m = (d.model || 'unknown').trim() || 'unknown'
  return `${m}|s${d.sliders}|b${d.buttons ?? 0}`
}

const LEGACY_PRESETS_KEY = 'idei-presets'
const PRESETS_BY_MODEL_KEY = 'idei-presets-by-model'

function migratePresetEntry(p: PresetEntry): PresetEntry {
  const sliderValues = emptySliderValues()
  const assignments = emptyAssignments()
  for (let i = 0; i < MAX_SLIDERS; i++) {
    sliderValues[i] = typeof p.sliderValues[i] === 'number' ? p.sliderValues[i] : 0
    assignments[i] = Array.isArray(p.assignments[i]) ? [...p.assignments[i]] : []
  }
  let buttonBindings = defaultButtonBindings()
  if (Array.isArray(p.buttonBindings) && p.buttonBindings.length === MEDIA_BUTTON_SLOTS) {
    const parsed = (p.buttonBindings as unknown[]).map(parseButtonBinding)
    if (parsed.every((b): b is ButtonBinding => b != null)) {
      buttonBindings = parsed
    }
  } else if (Array.isArray(p.buttonMediaActions) && p.buttonMediaActions.length === MEDIA_BUTTON_SLOTS) {
    buttonBindings = (p.buttonMediaActions as MediaKeyAction[]).map(mediaRowToBinding)
  }
  let buttonHwSliderMute = defaultHwSliderMute()
  if (Array.isArray(p.buttonHwSliderMute) && p.buttonHwSliderMute.length === MEDIA_BUTTON_SLOTS) {
    buttonHwSliderMute = [...p.buttonHwSliderMute]
  }
  let shortcutMuteLedMap = defaultShortcutMuteLedMap()
  if (Array.isArray(p.shortcutMuteLedMap) && p.shortcutMuteLedMap.length === MEDIA_BUTTON_SLOTS) {
    shortcutMuteLedMap = [...p.shortcutMuteLedMap]
  } else if (p.shortcutMuteLed === true) {
    shortcutMuteLedMap = Array.from({ length: MEDIA_BUTTON_SLOTS }, () => true)
  }
  return {
    ...p,
    sliderValues,
    assignments,
    buttonBindings,
    buttonHwSliderMute,
    shortcutMuteLedMap,
    buttonMediaActions: undefined,
    shortcutMuteLed: undefined,
  }
}

function loadPresetsByModel(): Record<string, PresetEntry[]> {
  try {
    const raw = localStorage.getItem(PRESETS_BY_MODEL_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as Record<string, unknown>
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        const out: Record<string, PresetEntry[]> = {}
        for (const [k, v] of Object.entries(parsed)) {
          if (Array.isArray(v)) {
            out[k] = v.map((p) => migratePresetEntry(p as PresetEntry))
          }
        }
        return out
      }
    }
  } catch {
    /* ignore */
  }
  try {
    const raw = localStorage.getItem(LEGACY_PRESETS_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as PresetEntry[]
      if (Array.isArray(parsed) && parsed.length > 0) {
        const migrated = { __legacy__: parsed.map(migratePresetEntry) }
        try {
          localStorage.setItem(PRESETS_BY_MODEL_KEY, JSON.stringify(migrated))
          localStorage.removeItem(LEGACY_PRESETS_KEY)
        } catch {
          /* ignore */
        }
        return migrated
      }
    }
  } catch {
    /* ignore */
  }
  return {}
}

function persistPresetsByModel(map: Record<string, PresetEntry[]>) {
  try {
    localStorage.setItem(PRESETS_BY_MODEL_KEY, JSON.stringify(map))
  } catch {
    /* ignore */
  }
}

/** Zamienia surowe komunikaty błędów na krótkie, zrozumiałe dla użytkownika. */
function formatConnectionError(raw: string): string {
  const s = raw.toLowerCase()
  if (s.includes('timeout') || s.includes('timed out')) return 'Connection timed out — check the USB cable and try again.'
  if (s.includes('access denied') || s.includes('odmowa dostępu') || s.includes('permission')) return 'Access denied to port — run the app with permissions or close other apps using the device.'
  if (s.includes('in use') || s.includes('używan') || s.includes('already open')) return 'Port is in use by another application. Close it or disconnect the device.'
  if (s.includes('not found') || s.includes('nie znaleziono') || s.includes('no device')) return 'Device not found — check the connection and select the port again.'
  if (s.includes('could not open') || s.includes('failed to open')) return 'Could not open port — check the USB cable and driver.'
  return raw || 'Connection error. Check the cable and port.'
}

function App() {
  const [ideiPorts, setIdeiPorts] = useState<string[]>([])
  const [selectedPort, setSelectedPort] = useState('')
  const [connectedDevices, setConnectedDevices] = useState<Record<string, DeviceInfo>>({})
  const [activePort, setActivePort] = useState<string | null>(null)
  const connectedDevicesRef = useRef<Record<string, DeviceInfo>>({})
  const [sliderValues, setSliderValues] = useState<number[]>(() => emptySliderValues())
  const [log, setLog] = useState<string[]>(['— no events'])
  const [connecting, setConnecting] = useState(false)
  const [scanning, setScanning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [assignments, setAssignments] = useState<VolumeTarget[][]>(() => emptyAssignments())
  // Inline error text shown on the specific slider card when an assignment is rejected.
  const [assignmentErrorBySlider, setAssignmentErrorBySlider] = useState<(string | null)[]>(
    () => Array.from({ length: MAX_SLIDERS }, () => null),
  )
  const [audioSessions, setAudioSessions] = useState<AudioSessionInfo[]>([])
  const [autostartEnabled, setAutostartEnabled] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [theme, setTheme] = useState<'dark' | 'light' | 'system'>('system')
  const [lang, setLang] = useState<Lang>(() => {
    try {
      const saved = localStorage.getItem(LANG_STORAGE_KEY)
      return saved === 'en' ? 'en' : 'pl'
    } catch {
      return 'pl'
    }
  })
  const [splashVisible, setSplashVisible] = useState(true)
  const [presetsByModel, setPresetsByModel] = useState<Record<string, PresetEntry[]>>(() => loadPresetsByModel())
  const [activePresetByModel, setActivePresetByModel] = useState<Record<string, string | null>>({})
  const [shortcutMuteLedMap, setShortcutMuteLedMap] = useState<boolean[]>(() => defaultShortcutMuteLedMap())
  const [buttonBindings, setButtonBindings] = useState<ButtonBinding[]>(() => defaultButtonBindings())
  const [buttonHwSliderMute, setButtonHwSliderMute] = useState<boolean[]>(() => defaultHwSliderMute())
  const rawValuesRef = useRef<number[]>(emptySliderValues())
  const perPortStateRef = useRef<Record<string, { sliderValues: number[]; assignments: VolumeTarget[][]; buttonBindings: ButtonBinding[]; buttonHwSliderMute: boolean[]; shortcutMuteLedMap: boolean[] }>>({})
  const perPortAssignmentsReadyRef = useRef<Record<string, boolean>>({})
  const device = useMemo(() => (activePort ? connectedDevices[activePort] ?? null : null), [connectedDevices, activePort])

  const parseComNumber = (p: string): number => {
    const m = /^COM(\d+)$/i.exec((p || '').trim())
    if (!m) return Number.POSITIVE_INFINITY
    return Number(m[1])
  }

  const sortComPorts = (ports: string[]): string[] => {
    return [...ports].sort((a, b) => {
      const na = parseComNumber(a)
      const nb = parseComNumber(b)
      if (na !== nb) return na - nb
      return a.localeCompare(b)
    })
  }

  const isGamePid = (pid: number) => {
    const s = audioSessions.find((x) => x.pid === pid)
    return !!s?.is_game
  }

  const isGamesCategory = (t: VolumeTarget) => t.type === 'category' && t.id === 'gry'

  /** Zgodne z backendem `audio::normalize_app_name` — ten sam proces ma inny PID po restarcie. */
  const normalizeAppNameForConflict = (name: string | undefined): string => {
    const s = (name ?? '').trim().toLowerCase()
    if (!s || s.startsWith('pid ')) return ''
    return s.replace(/\.exe$/i, '').trim()
  }

  const targetsConflict = (a: VolumeTarget, b: VolumeTarget): boolean => {
    if (a.type === 'system' && b.type === 'system') return true
    if (a.type === 'mic' && b.type === 'mic') return true
    if (a.type === 'app' && b.type === 'app') {
      if (a.pid === b.pid) return true
      const na = normalizeAppNameForConflict(a.name)
      const nb = normalizeAppNameForConflict(b.name)
      return na.length > 0 && na === nb
    }
    if (isGamesCategory(a) && b.type === 'app') return isGamePid(b.pid)
    if (a.type === 'app' && isGamesCategory(b)) return isGamePid(a.pid)
    if (isGamesCategory(a) && isGamesCategory(b)) return true
    return false
  }

  const conflictReason = (
    sliderIndex: number,
    nextTargets: VolumeTarget[],
    prevAssignments: VolumeTarget[][],
    lang: 'pl' | 'en',
  ) => {
    for (const t of nextTargets) {
      for (let j = 0; j < prevAssignments.length; j++) {
        if (j === sliderIndex) continue
        for (const u of prevAssignments[j] ?? []) {
          if (!targetsConflict(t, u)) continue
          if (isGamesCategory(t) && isGamesCategory(u)) {
            return lang === 'pl' ? 'Grupa Games jest już na innym suwaku.' : 'Games group is already on another slider.'
          }
          if (isGamesCategory(t) && u.type === 'app') {
            return lang === 'pl' ? 'Ten tytuł jest objęty grupą Games na innym suwaku.' : 'This title is already part of Games on another slider.'
          }
          if (t.type === 'app' && isGamesCategory(u)) {
            return lang === 'pl' ? 'Ta aplikacja jest objęta grupą Games na innym suwaku.' : 'This app is already part of Games on another slider.'
          }
          if (t.type === 'app' && u.type === 'app') {
            return lang === 'pl' ? 'Ta aplikacja jest już na innym suwaku.' : 'This app is already used on another slider.'
          }
          return lang === 'pl' ? 'Konflikt przypisań na innym suwaku.' : 'Assignment conflict on another slider.'
        }
      }
    }
    return null
  }

  const conflictReasonAcrossDevices = (
    currentPort: string | null,
    sliderIndex: number,
    nextTargets: VolumeTarget[],
    currentAssignments: VolumeTarget[][],
    lang: 'pl' | 'en',
  ) => {
    const ports = Object.keys(connectedDevices)
    for (const port of ports) {
      if (currentPort && port === currentPort) continue
      if (!perPortAssignmentsReadyRef.current[port]) {
        return lang === 'pl'
          ? 'Synchronizuję stan drugiego urządzenia. Spróbuj ponownie za chwilę.'
          : 'Synchronizing state of another device. Try again in a moment.'
      }
    }
    for (const t of nextTargets) {
      for (const port of ports) {
        const portAssignments =
          port === currentPort
            ? currentAssignments
            : perPortStateRef.current[port]?.assignments ?? emptyAssignments()
        for (let j = 0; j < portAssignments.length; j++) {
          if (port === currentPort && j === sliderIndex) continue
          for (const u of portAssignments[j] ?? []) {
            if (!targetsConflict(t, u)) continue
            if (isGamesCategory(t) && isGamesCategory(u)) {
              return lang === 'pl' ? 'Grupa Games jest już przypisana na innym urządzeniu.' : 'Games group is already assigned on another device.'
            }
            if (isGamesCategory(t) && u.type === 'app') {
              return lang === 'pl' ? 'Ten tytuł jest już objęty grupą Games na innym urządzeniu.' : 'This title is already part of Games on another device.'
            }
            if (t.type === 'app' && isGamesCategory(u)) {
              return lang === 'pl' ? 'Ta aplikacja jest objęta grupą Games na innym urządzeniu.' : 'This app is already part of Games on another device.'
            }
            if (t.type === 'app' && u.type === 'app') {
              return lang === 'pl' ? 'Ta aplikacja jest już przypisana na innym urządzeniu.' : 'This app is already assigned on another device.'
            }
            return lang === 'pl' ? 'Konflikt przypisań na innym urządzeniu.' : 'Assignment conflict on another device.'
          }
        }
      }
    }
    return null
  }

  const deviceKey = useMemo(() => (device ? devicePresetKey(device) : null), [device])
  const presets = useMemo(
    () => (deviceKey ? presetsByModel[deviceKey] ?? [] : []),
    [presetsByModel, deviceKey],
  )
  const activePreset = useMemo(
    () => (deviceKey ? activePresetByModel[deviceKey] ?? null : null),
    [activePresetByModel, deviceKey],
  )

  /** Jednorazowa migracja starych presetów z `__legacy__` do klucza bieżącego urządzenia. */
  useEffect(() => {
    if (!device) return
    const key = devicePresetKey(device)
    setPresetsByModel((m) => {
      const cur = m[key]
      if (cur && cur.length > 0) return m
      const legacy = m.__legacy__
      if (legacy && legacy.length > 0) {
        const next: Record<string, PresetEntry[]> = { ...m, [key]: legacy.map(migratePresetEntry) }
        delete next.__legacy__
        persistPresetsByModel(next)
        return next
      }
      return m
    })
  }, [device])

  const applyPreset = useCallback(
    (preset: PresetEntry) => {
      if (!deviceKey) return
      const m = migratePresetEntry(preset)
      setSliderValues(m.sliderValues)
      // Prevent duplicated app/game targets across sliders (avoids volume "fighting").
      // Keep the first occurrence (lowest slider index) and skip conflicting entries in later sliders.
      const sanitizedAssignments: VolumeTarget[][] = Array.from({ length: MAX_SLIDERS }, () => [])
      for (let i = 0; i < MAX_SLIDERS; i++) {
        const kept: VolumeTarget[] = []
        for (const t of m.assignments[i]) {
          let conflict = false
          for (let j = 0; j < i && !conflict; j++) {
            for (const u of sanitizedAssignments[j]) {
              if (targetsConflict(t, u)) {
                conflict = true
                break
              }
            }
          }
          if (!conflict) kept.push(t)
        }
        sanitizedAssignments[i] = kept
      }

      setAssignments(sanitizedAssignments)
      rawValuesRef.current = m.sliderValues
      setButtonBindings(m.buttonBindings ?? defaultButtonBindings())
      setButtonHwSliderMute(m.buttonHwSliderMute ?? defaultHwSliderMute())
      const smap = m.shortcutMuteLedMap ?? defaultShortcutMuteLedMap()
      setShortcutMuteLedMap(smap)
      setActivePresetByModel((prev) => ({ ...prev, [deviceKey]: preset.id }))
      for (let i = 0; i < MAX_SLIDERS; i++) {
        if (activePort) invoke('set_volume_assignment', { portName: activePort, sliderIndex: i, targets: sanitizedAssignments[i] }).catch(() => {})
      }
      if (activePort) invoke('set_button_bindings', { portName: activePort, bindings: m.buttonBindings ?? defaultButtonBindings() }).catch(() => {})
      if (activePort) invoke('set_button_hw_slider_mute', { portName: activePort, enabled: m.buttonHwSliderMute ?? defaultHwSliderMute() }).catch(() => {})
      if (activePort) invoke('set_shortcut_mute_led_map', { portName: activePort, enabled: smap }).catch(() => {})
    },
    [deviceKey, targetsConflict, activePort],
  )

  const saveCurrentToPreset = useCallback(
    (presetId: string) => {
      if (!deviceKey) return
      setPresetsByModel((m) => {
        const list = m[deviceKey] ?? []
        const idx = list.findIndex((p) => p.id === presetId)
        if (idx < 0) return m
        const prev = list[idx]
        const updated: PresetEntry = {
          ...prev,
          sliderValues: [...sliderValues],
          assignments: assignments.map((a) => [...a]),
          buttonBindings: [...buttonBindings],
          buttonHwSliderMute: [...buttonHwSliderMute],
          shortcutMuteLedMap: [...shortcutMuteLedMap],
        }
        const nextList = list.slice()
        nextList[idx] = updated
        const out = { ...m, [deviceKey]: nextList }
        persistPresetsByModel(out)
        return out
      })
    },
    [deviceKey, sliderValues, assignments, buttonBindings, buttonHwSliderMute, shortcutMuteLedMap],
  )

  const addPreset = useCallback(() => {
    if (!deviceKey) return
    const entry: PresetEntry = {
      id: crypto.randomUUID(),
      name: 'New preset',
      sliderValues: [...sliderValues],
      assignments: assignments.map((a) => [...a]),
      buttonBindings: [...buttonBindings],
      buttonHwSliderMute: [...buttonHwSliderMute],
      shortcutMuteLedMap: [...shortcutMuteLedMap],
    }
    setPresetsByModel((m) => {
      const list = m[deviceKey] ?? []
      const nextList = [...list, entry]
      const out = { ...m, [deviceKey]: nextList }
      persistPresetsByModel(out)
      return out
    })
    setActivePresetByModel((prev) => ({ ...prev, [deviceKey]: entry.id }))
  }, [deviceKey, sliderValues, assignments, buttonBindings, buttonHwSliderMute, shortcutMuteLedMap])

  const renamePreset = useCallback(
    (id: string, name: string) => {
      if (!deviceKey) return
      setPresetsByModel((m) => {
        const list = m[deviceKey] ?? []
        const nextList = list.map((p) => (p.id === id ? { ...p, name: name.trim() || p.name } : p))
        const out = { ...m, [deviceKey]: nextList }
        persistPresetsByModel(out)
        return out
      })
    },
    [deviceKey],
  )

  const deletePreset = useCallback(
    (id: string) => {
      if (!deviceKey) return
      setPresetsByModel((m) => {
        const list = m[deviceKey] ?? []
        const nextList = list.filter((p) => p.id !== id)
        const out = { ...m, [deviceKey]: nextList }
        persistPresetsByModel(out)
        return out
      })
      setActivePresetByModel((prev) => {
        if (prev[deviceKey] !== id) return prev
        return { ...prev, [deviceKey]: null }
      })
    },
    [deviceKey],
  )

  const runScan = useCallback(() => {
    setScanning(true)
    invoke('start_scan_idei_ports').catch(() => setScanning(false))
  }, [])

  const saveActivePortState = useCallback(() => {
    if (!activePort) return
    perPortStateRef.current[activePort] = {
      sliderValues: [...sliderValues],
      assignments: assignments.map((a) => [...a]),
      buttonBindings: [...buttonBindings],
      buttonHwSliderMute: [...buttonHwSliderMute],
      shortcutMuteLedMap: [...shortcutMuteLedMap],
    }
  }, [activePort, sliderValues, assignments, buttonBindings, buttonHwSliderMute, shortcutMuteLedMap])

  const loadPortState = useCallback((port: string, applyToUi = false) => {
    const local = perPortStateRef.current[port]
    if (local && applyToUi) {
      setSliderValues(local.sliderValues)
      setAssignments(local.assignments)
      setButtonBindings(local.buttonBindings)
      setButtonHwSliderMute(local.buttonHwSliderMute)
      setShortcutMuteLedMap(local.shortcutMuteLedMap)
      rawValuesRef.current = [...local.sliderValues]
    }
    invoke('get_slider_values', { portName: port }).then((v: unknown) => {
      if (Array.isArray(v)) {
        const next = Array.from({ length: MAX_SLIDERS }, (_, i) => Number(v[i] ?? 0))
        const cur = perPortStateRef.current[port]
        perPortStateRef.current[port] = {
          sliderValues: next,
          assignments: cur?.assignments ?? emptyAssignments(),
          buttonBindings: cur?.buttonBindings ?? defaultButtonBindings(),
          buttonHwSliderMute: cur?.buttonHwSliderMute ?? defaultHwSliderMute(),
          shortcutMuteLedMap: cur?.shortcutMuteLedMap ?? defaultShortcutMuteLedMap(),
        }
        if (applyToUi) {
          setSliderValues(next)
          rawValuesRef.current = [...next]
        }
      }
    }).catch(() => {})
    invoke('get_volume_assignments', { portName: port }).then((a: unknown) => {
      if (Array.isArray(a)) {
        const nextAssignments = Array.from(
          { length: MAX_SLIDERS },
          (_, i) => (Array.isArray(a[i]) ? (a[i] as VolumeTarget[]) : []),
        )
        const cur = perPortStateRef.current[port]
        perPortStateRef.current[port] = {
          sliderValues: cur?.sliderValues ?? emptySliderValues(),
          assignments: nextAssignments,
          buttonBindings: cur?.buttonBindings ?? defaultButtonBindings(),
          buttonHwSliderMute: cur?.buttonHwSliderMute ?? defaultHwSliderMute(),
          shortcutMuteLedMap: cur?.shortcutMuteLedMap ?? defaultShortcutMuteLedMap(),
        }
        if (applyToUi) {
          setAssignments(nextAssignments)
        }
        perPortAssignmentsReadyRef.current[port] = true
      }
    }).catch(() => {})
    invoke('get_button_bindings', { portName: port }).then((list: unknown) => {
      if (Array.isArray(list) && list.length === MEDIA_BUTTON_SLOTS) {
        const parsed = list.map(parseButtonBinding)
        if (parsed.every((b): b is ButtonBinding => b != null)) {
          const cur = perPortStateRef.current[port]
          perPortStateRef.current[port] = {
            sliderValues: cur?.sliderValues ?? emptySliderValues(),
            assignments: cur?.assignments ?? emptyAssignments(),
            buttonBindings: parsed,
            buttonHwSliderMute: cur?.buttonHwSliderMute ?? defaultHwSliderMute(),
            shortcutMuteLedMap: cur?.shortcutMuteLedMap ?? defaultShortcutMuteLedMap(),
          }
          if (applyToUi) setButtonBindings(parsed)
        }
      }
    }).catch(() => {})
    invoke('get_button_hw_slider_mute', { portName: port }).then((list: unknown) => {
      if (Array.isArray(list) && list.length === MEDIA_BUTTON_SLOTS) {
        const next = list as boolean[]
        const cur = perPortStateRef.current[port]
        perPortStateRef.current[port] = {
          sliderValues: cur?.sliderValues ?? emptySliderValues(),
          assignments: cur?.assignments ?? emptyAssignments(),
          buttonBindings: cur?.buttonBindings ?? defaultButtonBindings(),
          buttonHwSliderMute: next,
          shortcutMuteLedMap: cur?.shortcutMuteLedMap ?? defaultShortcutMuteLedMap(),
        }
        if (applyToUi) setButtonHwSliderMute(next)
      }
    }).catch(() => {})
    invoke('get_shortcut_mute_led_map', { portName: port }).then((list: unknown) => {
      if (Array.isArray(list) && list.length === MEDIA_BUTTON_SLOTS) {
        const next = list as boolean[]
        const cur = perPortStateRef.current[port]
        perPortStateRef.current[port] = {
          sliderValues: cur?.sliderValues ?? emptySliderValues(),
          assignments: cur?.assignments ?? emptyAssignments(),
          buttonBindings: cur?.buttonBindings ?? defaultButtonBindings(),
          buttonHwSliderMute: cur?.buttonHwSliderMute ?? defaultHwSliderMute(),
          shortcutMuteLedMap: next,
        }
        if (applyToUi) setShortcutMuteLedMap(next)
      }
    }).catch(() => {})
  }, [])

  const connectToPort = useCallback(async (portName: string) => {
    setConnecting(true)
    setError(null)
    try {
      await invoke('connect_serial', { portName })
      // Urządzenie ustawiane w evencie device-connected (handshake w tle)
    } catch (e) {
      setError(formatConnectionError(String(e)))
      setConnecting(false)
    }
    // setConnecting(false) przy device-connected lub connect-error (w useEffect)
  }, [])

  const handleConnect = useCallback(async () => {
    if (!selectedPort) return
    await connectToPort(selectedPort)
  }, [selectedPort, connectToPort])

  const handleDisconnect = useCallback(async () => {
    const targetPort =
      selectedPort && connectedDevices[selectedPort]
        ? selectedPort
        : activePort
    if (!targetPort) return
    saveActivePortState()
    try {
      await invoke('disconnect_serial', { portName: targetPort })
    } catch (e) {
      setError(formatConnectionError(String(e)))
    }
  }, [selectedPort, connectedDevices, activePort, saveActivePortState])

  useEffect(() => {
    runScan()
  }, [runScan])

  useEffect(() => {
    connectedDevicesRef.current = connectedDevices
  }, [connectedDevices])

  useEffect(() => {
    invoke<DeviceInfo[]>('get_connected_devices')
      .then((list) => {
        const map: Record<string, DeviceInfo> = {}
        for (const d of list || []) map[d.port] = d
        setConnectedDevices(map)
        if (!activePort && list.length > 0) setActivePort(list[0].port)
      })
      .catch(() => {})
  }, [activePort])

  // Po skanie: jeśli mamy zapisany last_port i jest na liście, ustaw go jako wybrany
  useEffect(() => {
    if (ideiPorts.length === 0) return
    invoke<{ last_port?: string }>('get_app_config')
      .then((cfg) => {
        if (cfg?.last_port && ideiPorts.includes(cfg.last_port)) {
          setSelectedPort(cfg.last_port)
        }
      })
      .catch(() => {})
  }, [ideiPorts])

  // Stan autostartu (plugin)
  useEffect(() => {
    isEnabled().then(setAutostartEnabled).catch(() => {})
  }, [])

  // Theme: odczyt z localStorage, zastosowanie
  useEffect(() => {
    const saved = localStorage.getItem('idei-theme') as 'dark' | 'light' | 'system' | null
    if (saved === 'dark' || saved === 'light' || saved === 'system') setTheme(saved)
  }, [])
  // Language persistence
  useEffect(() => {
    try {
      localStorage.setItem(LANG_STORAGE_KEY, lang)
    } catch {
      /* ignore */
    }
  }, [lang])
  const isDark = theme === 'dark' || (theme === 'system' && typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches)
  useEffect(() => {
    localStorage.setItem('idei-theme', theme)
  }, [theme])

  // Splash: ukryj po 1.5 s
  useEffect(() => {
    const t = setTimeout(() => setSplashVisible(false), 1500)
    return () => clearTimeout(t)
  }, [])

  // Telemetria MVP: start aplikacji.
  useEffect(() => {
    trackAppStartedOnce()
  }, [])

  const loadAudioSessions = useCallback(() => {
    invoke<AudioSessionInfo[]>('get_audio_sessions')
      .then((s) => setAudioSessions(Array.isArray(s) ? s : []))
      .catch(() => setAudioSessions([]))
  }, [])

  const prevActivePortRef = useRef<string | null>(null)
  useEffect(() => {
    const prev = prevActivePortRef.current
    if (prev && prev !== activePort) {
      perPortStateRef.current[prev] = {
        sliderValues: [...sliderValues],
        assignments: assignments.map((a) => [...a]),
        buttonBindings: [...buttonBindings],
        buttonHwSliderMute: [...buttonHwSliderMute],
        shortcutMuteLedMap: [...shortcutMuteLedMap],
      }
    }
    if (activePort) loadPortState(activePort, true)
    prevActivePortRef.current = activePort
  }, [activePort, loadPortState])

  // Poll wartości z urządzenia (0–1023, deej); zapis do ref dla wygładzania
  useEffect(() => {
    if (!device || device.sliders < 1) return
    const n = device.sliders
    const t = setInterval(async () => {
      try {
        const v = (await invoke('get_slider_values', { portName: device.port })) as number[]
        if (Array.isArray(v) && v.length >= n) {
          rawValuesRef.current = Array.from({ length: MAX_SLIDERS }, (_, i) =>
            i < n ? Math.min(1023, Math.max(0, Number(v[i]) ?? 0)) : 0,
          )
        }
      } catch {
        // ignore (e.g. disconnected)
      }
    }, 50)
    return () => clearInterval(t)
  }, [device])

  // Wygładzanie suwaków (lerp do wartości z urządzenia) — płynny ruch zamiast skoków
  useEffect(() => {
    if (!device || device.sliders < 1) return
    const n = device.sliders
    let rafId: number
    const step = () => {
      setSliderValues((prev) => {
        const raw = rawValuesRef.current
        const next = Array.from({ length: MAX_SLIDERS }, (_, i) => {
          if (i >= n) return 0
          const p = prev[i] ?? 0
          const r = raw[i] ?? 0
          return p + (r - p) * 0.32
        })
        const settled = Array.from({ length: n }, (_, i) =>
          Math.abs((next[i] ?? 0) - (raw[i] ?? 0)) < 0.5,
        ).every(Boolean)
        return settled
          ? Array.from({ length: MAX_SLIDERS }, (_, i) => (i < n ? raw[i] ?? 0 : 0))
          : next
      })
      rafId = requestAnimationFrame(step)
    }
    rafId = requestAnimationFrame(step)
    return () => cancelAnimationFrame(rafId)
  }, [device])

  useEffect(() => {
    const unlistenScan = listen<string[]>('scan-complete', (e) => {
      setScanning(false)
      const list = Array.isArray(e.payload) ? e.payload : []
      // Keep already-connected ports visible even if scan can't reopen them right now.
      const connected = Object.keys(connectedDevicesRef.current)
      const merged = sortComPorts(Array.from(new Set([...list, ...connected])))
      setIdeiPorts(merged)
      if (list.length > 0) {
        setSelectedPort((prev) => {
          if (prev && merged.includes(prev)) return prev
          return merged[0] ?? ''
        })
        if (list.length === 1) {
          connectToPort(list[0])
        }
      } else {
        setSelectedPort((prev) => (prev && merged.includes(prev) ? prev : merged[0] ?? ''))
      }
    })
    const unlistenConnected = listen<DeviceInfo>('device-connected-port', (e) => {
      const d = e.payload
      void trackTelemetryEvent('device_connected', { port: d.port, model: d.model, sliders: d.sliders })
      setConnectedDevices((prev) => ({ ...prev, [d.port]: d }))
      setSelectedPort(d.port)
      setActivePort(d.port)
      perPortAssignmentsReadyRef.current[d.port] = false
      setIdeiPorts((prev) => sortComPorts(Array.from(new Set([...prev, d.port]))))
      setError(null)
      setConnecting(false)
      loadPortState(d.port, false)
    })
    const unlistenReconciled = listen<string[]>('assignments-reconciled', (e) => {
      const ports = Array.isArray(e.payload) ? e.payload : []
      const ap = activePort
      for (const p of ports) {
        loadPortState(p, ap === p)
      }
    })
    const unlistenConnectError = listen<string>('connect-error', (e) => {
      void trackTelemetryEvent('serial_error', { message: String(e.payload ?? 'Connection error') })
      setError(formatConnectionError(e.payload ?? 'Connection error'))
      setConnecting(false)
    })
    const unlistenDisconnect = listen<string>('device-disconnected-port', async (e) => {
      const port = e.payload
      void trackTelemetryEvent('device_disconnected', { port })
      setConnectedDevices((prev) => {
        const next = { ...prev }
        delete next[port]
        return next
      })
      delete perPortAssignmentsReadyRef.current[port]
      delete perPortStateRef.current[port]
      setActivePort((prev) => {
        if (prev !== port) return prev
        const keys = Object.keys(connectedDevicesRef.current).filter((k) => k !== port)
        const nextPort = sortComPorts(keys)[0]
        return nextPort ?? null
      })
      if (activePort === port) {
        const z = emptySliderValues()
        setSliderValues(z)
        rawValuesRef.current = [...z]
      }
    })
    const unlistenSliders = listen<{ port: string; values: number[] }>('slider-values-port', (e) => {
      const p = e.payload?.port
      const v = e.payload?.values
      if (!p || !Array.isArray(v)) return
      const next = Array.from({ length: MAX_SLIDERS }, (_, i) => (i < v.length ? Number(v[i]) || 0 : 0))
      const cur = perPortStateRef.current[p]
      perPortStateRef.current[p] = {
        sliderValues: next,
        assignments: cur?.assignments ?? emptyAssignments(),
        buttonBindings: cur?.buttonBindings ?? defaultButtonBindings(),
        buttonHwSliderMute: cur?.buttonHwSliderMute ?? defaultHwSliderMute(),
        shortcutMuteLedMap: cur?.shortcutMuteLedMap ?? defaultShortcutMuteLedMap(),
      }
      if (activePort === p) setSliderValues(next)
    })
    const unlistenLog = listen<string>('log', (e) => {
      setLog((prev) => [e.payload, ...prev.slice(0, 49)])
    })
    const unlistenBtn = listen('device-button', (e: { payload: { index: number; seq: number } }) => {
      const p = e.payload
      const line = `Button ${p.index} (seq ${p.seq})`
      setLog((prev) => [line, ...prev.slice(0, 49)])
    })
    return () => {
      unlistenScan.then((u) => u())
      unlistenConnected.then((u) => u())
      unlistenReconciled.then((u) => u())
      unlistenConnectError.then((u) => u())
      unlistenDisconnect.then((u) => u())
      unlistenSliders.then((u) => u())
      unlistenLog.then((u) => u())
      unlistenBtn.then((u) => u())
    }
  }, [connectToPort, loadPortState, activePort, connectedDevices])

  const isSelectedConnected = !!(selectedPort && connectedDevices[selectedPort])

  return (
    <div className={`app-container flex flex-col h-full min-h-full overflow-hidden rounded-2xl ${isDark ? 'dark' : 'light'}`}>
      {splashVisible && (
        <div className="fixed inset-0 z-[200] flex items-center justify-center rounded-2xl bg-background">
          <div className="flex flex-col items-center gap-4">
            <div className="w-16 h-16 rounded-2xl bg-accent/20 flex items-center justify-center">
              <span className="text-3xl"><img src='/assets/logo.png'></img></span>
            </div>
            <span className="text-lg font-medium text-foreground">IDEI Control</span>
            <span className="text-sm text-muted-foreground">Loading…</span>
          </div>
        </div>
      )}

      <TitleBar />
      <div className="flex-1 min-h-0 overflow-auto p-6">
        <div className="mx-auto max-w-7xl space-y-6">
          {/* Pasek statusu (v0) */}
          <div className="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-border bg-card p-4">
            <div className="flex items-center gap-4">
              <div className="flex items-center gap-3">
                <div className={`h-3 w-3 rounded-full ${device ? 'bg-accent' : 'bg-muted-foreground'} ${device ? 'animate-pulse' : ''}`} />
                <div>
                  <p className="text-sm font-medium text-foreground">
                    {device ? 'Connected' : 'Disconnected'}
                  </p>
                  {device && (
                    <p className="text-xs text-muted-foreground">
                      {device.port}
                      {device.fw != null && device.fw !== '' && (
                        <span className="ml-2 opacity-80">· fw {device.fw}</span>
                      )}
                    </p>
                  )}
                </div>
              </div>
              {error && (
                <p className="text-xs text-destructive break-words max-w-xs" role="alert">{error}</p>
              )}
            </div>

            <div className="flex items-center gap-3">
              <select
                value={selectedPort}
                onChange={(e) => {
                  const port = e.target.value
                  setSelectedPort(port)
                  if (connectedDevices[port]) {
                    setActivePort(port)
                  }
                }}
                disabled={scanning}
                className="rounded-lg border border-border bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-accent focus:ring-1 focus:ring-accent"
              >
                  {ideiPorts.length === 0 && !scanning && <option value="">— none found —</option>}
                  {scanning && ideiPorts.length === 0 && <option value="">Scanning…</option>}
                  {ideiPorts.map((name) => {
                    const info = connectedDevices[name]
                    const label = info ? `${info.model} · ${name}` : name
                    return (
                      <option key={name} value={name}>
                        {label}
                      </option>
                    )
                  })}
              </select>
              <button
                type="button"
                onClick={handleConnect}
                disabled={!selectedPort || connecting || scanning || isSelectedConnected}
                className="rounded-lg px-4 py-2 text-sm font-medium transition-colors bg-accent text-accent-foreground hover:bg-accent/90 disabled:opacity-50"
              >
                {connecting ? 'Connecting…' : isSelectedConnected ? 'Connected' : 'Connect'}
              </button>
              <button
                type="button"
                onClick={handleDisconnect}
                disabled={!activePort || connecting}
                className="rounded-lg px-4 py-2 text-sm font-medium transition-colors bg-destructive text-destructive-foreground hover:bg-destructive/90 disabled:opacity-50"
              >
                Disconnect
              </button>
              <button
                type="button"
                onClick={runScan}
                disabled={scanning}
                className="rounded-lg border border-border bg-card p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-50"
              >
                <IconRefresh className="h-5 w-5" />
              </button>
              <button
                type="button"
                onClick={() => setSettingsOpen(true)}
                className="rounded-lg border border-border bg-card p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              >
                <IconSettings className="h-5 w-5" />
              </button>
            </div>
          </div>
          {/* Karty suwaków (v0) — items-start żeby karty nie rozciągały się na równą wysokość */}
          <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5 items-start">
            {device && device.sliders > 0
              ? Array.from({ length: device.sliders }, (_, i) => (
                  <V0ControllerCard
                    key={i}
                    index={i + 1}
                    sliderIndex={i}
                    lang={lang}
                    showButtonMapping={i < MEDIA_BUTTON_SLOTS}
                    binding={buttonBindings[i] ?? { kind: 'none' }}
                    onBindingChange={(b) => {
                      const next = [...buttonBindings]
                      while (next.length < MEDIA_BUTTON_SLOTS) next.push({ kind: 'none' })
                      next[i] = b
                      setButtonBindings(next)
                      if (activePort) invoke('set_button_bindings', { portName: activePort, bindings: next }).catch(() => {})
                    }}
                    hwSliderMute={!!buttonHwSliderMute[i]}
                    onHwSliderMuteChange={(v) => {
                      const next = [...buttonHwSliderMute]
                      while (next.length < MEDIA_BUTTON_SLOTS) next.push(false)
                      next[i] = v
                      setButtonHwSliderMute(next)
                      if (activePort) invoke('set_button_hw_slider_mute', { portName: activePort, enabled: next }).catch(() => {})
                    }}
                    shortcutNeoOnShortcut={!!shortcutMuteLedMap[i]}
                    onShortcutNeoChange={(v) => {
                      const next = [...shortcutMuteLedMap]
                      while (next.length < MEDIA_BUTTON_SLOTS) next.push(false)
                      next[i] = v
                      setShortcutMuteLedMap(next)
                      if (activePort) invoke('set_shortcut_mute_led_map', { portName: activePort, enabled: next }).catch(() => {})
                    }}
                    value={sliderValues[i] ?? 0}
                    targets={assignments[i] ?? []}
                    assignmentError={assignmentErrorBySlider[i] ?? null}
                    audioSessions={audioSessions}
                    loadAudioSessions={loadAudioSessions}
                    onAssignmentsChange={(targets) => {
                      const reason =
                        conflictReason(i, targets, assignments, lang) ??
                        conflictReasonAcrossDevices(activePort, i, targets, assignments, lang)
                      if (reason) {
                        setLog((prev) => [reason, ...prev.slice(0, 49)])
                        setAssignmentErrorBySlider((prev) => {
                          const next = [...prev]
                          next[i] = reason
                          return next
                        })
                        return false
                      }
                      setAssignmentErrorBySlider((prev) => {
                        const next = [...prev]
                        next[i] = null
                        return next
                      })
                      setAssignments((prev) => {
                        const next = [...prev]
                        next[i] = targets
                        if (activePort) {
                          const cur = perPortStateRef.current[activePort]
                          perPortStateRef.current[activePort] = {
                            sliderValues: cur?.sliderValues ?? [...sliderValues],
                            assignments: next.map((a) => [...a]),
                            buttonBindings: cur?.buttonBindings ?? [...buttonBindings],
                            buttonHwSliderMute: cur?.buttonHwSliderMute ?? [...buttonHwSliderMute],
                            shortcutMuteLedMap: cur?.shortcutMuteLedMap ?? [...shortcutMuteLedMap],
                          }
                        }
                        return next
                      })
                      if (activePort) invoke('set_volume_assignment', { portName: activePort, sliderIndex: i, targets }).catch(() => {})
                      return true
                    }}
                  />
                ))
              : null}
          </div>

          {/* Presety + karta urządzenia (v0) */}
          <div className="grid gap-6 lg:grid-cols-3">
            <div className="rounded-xl border border-border bg-card p-6 shadow-sm lg:col-span-2">
              <h3 className="text-sm font-medium uppercase tracking-wide text-muted-foreground mb-1">
                Presets
              </h3>
              <p className="text-xs text-muted-foreground mb-4">
                Presets are stored per device model (sliders / buttons). Connect your hardware to see and edit its list. Load applies sliders, assignments, and per-slider button mapping; &quot;Save here&quot; overwrites that preset.
              </p>
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
                {presets.map((preset) => {
                  const count = preset.assignments.reduce((n, row) => n + row.length, 0)
                  const isLoaded = activePreset === preset.id
                  return (
                    <div
                      key={preset.id}
                      className={`rounded-xl border p-4 text-left transition-all ${
                        isLoaded ? 'border-accent bg-accent/10' : 'border-border bg-background hover:border-accent/50'
                      } ${!device ? 'opacity-60' : ''}`}
                    >
                      <div className="flex items-center justify-between gap-2 mb-2">
                        <input
                          type="text"
                          value={preset.name}
                          onChange={(e) => renamePreset(preset.id, e.target.value)}
                          placeholder="Preset name"
                          disabled={!device}
                          className="flex-1 min-w-0 rounded border border-transparent bg-transparent px-1.5 py-0.5 text-base font-semibold text-foreground outline-none placeholder:text-muted-foreground focus:border-border focus:bg-background disabled:cursor-not-allowed"
                          onClick={(e) => e.stopPropagation()}
                        />
                        <button
                          type="button"
                          onClick={(e) => { e.stopPropagation(); deletePreset(preset.id) }}
                          disabled={!device}
                          className="shrink-0 rounded p-1 text-muted-foreground hover:text-destructive hover:bg-destructive/10 disabled:pointer-events-none disabled:opacity-50"
                          aria-label="Delete preset"
                        >
                          <IconX className="h-4 w-4" />
                        </button>
                      </div>
                      <p className="text-xs text-muted-foreground mb-3">
                        {count} assignment{count !== 1 ? 's' : ''}
                      </p>
                      <div className="flex gap-2">
                        <button
                          type="button"
                          onClick={() => applyPreset(preset)}
                          disabled={!device}
                          className={`flex-1 rounded-lg px-3 py-2 text-sm font-medium transition-colors disabled:opacity-50 disabled:pointer-events-none ${
                            isLoaded
                              ? 'bg-accent text-accent-foreground cursor-default'
                              : 'bg-accent text-accent-foreground hover:opacity-90'
                          }`}
                        >
                          {isLoaded ? 'Loaded' : 'Load'}
                        </button>
                        <button
                          type="button"
                          onClick={() => saveCurrentToPreset(preset.id)}
                          disabled={!device}
                          className="flex-1 rounded-lg border border-border bg-background px-3 py-2 text-sm text-foreground transition-colors hover:bg-muted disabled:opacity-50 disabled:pointer-events-none"
                        >
                          Save here
                        </button>
                      </div>
                    </div>
                  )
                })}
                <button
                  type="button"
                  onClick={addPreset}
                  disabled={!device}
                  className="flex flex-col items-center justify-center gap-2 rounded-xl border-2 border-dashed border-border bg-background/50 p-6 text-muted-foreground transition-colors hover:border-accent hover:text-accent hover:bg-accent/5 min-h-[140px] disabled:opacity-50 disabled:pointer-events-none"
                >
                  <span className="text-2xl">+</span>
                  <span className="text-sm font-medium">Save current as new preset</span>
                </button>
              </div>
            </div>

            <div className="rounded-xl border border-border bg-card p-6 shadow-sm">
              <h3 className="mb-4 text-sm font-medium uppercase tracking-wide text-muted-foreground">
                Connected device
              </h3>
              {device ? (
                <div className="space-y-4">
                  <div className="flex items-center gap-4">
                    <div className="rounded-lg bg-accent/10 p-3">
                      <IconCpu className="h-8 w-8 text-accent" />
                    </div>
                    <div>
                      <div className="font-semibold text-foreground">{device.model}</div>
                      <div className="text-xs text-muted-foreground">{device.port}</div>
                    </div>
                  </div>
                  <div className="space-y-2 border-t border-border pt-4 text-sm">
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Port</span>
                      <span className="font-mono text-foreground">{device.port}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Sliders</span>
                      <span className="font-mono text-foreground">{device.sliders}</span>
                    </div>
                    {device.proto != null && (
                      <div className="flex justify-between">
                        <span className="text-muted-foreground">Protocol</span>
                        <span className="font-mono text-foreground">{device.proto}</span>
                      </div>
                    )}
                    {device.fw != null && device.fw !== '' && (
                      <div className="flex justify-between">
                        <span className="text-muted-foreground">Firmware</span>
                        <span className="font-mono text-foreground">{device.fw}</span>
                      </div>
                    )}
                    {device.buttons != null && (
                      <div className="flex justify-between">
                        <span className="text-muted-foreground">Buttons</span>
                        <span className="font-mono text-foreground">{device.buttons}</span>
                      </div>
                    )}
                  </div>
                </div>
                ) : (
                <div className="flex min-h-[200px] items-center justify-center">
                  <div className="text-center">
                    <div className="mx-auto mb-3 rounded-lg bg-muted p-4">
                      <IconCpu className="h-12 w-12 text-muted-foreground" />
                    </div>
                    <p className="text-sm text-muted-foreground">No device connected</p>
                    <p className="mt-1 text-xs text-muted-foreground">Select a port and click Connect</p>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Modal ustawień (v0) */}
      {settingsOpen && (
        <div className="fixed inset-0 z-[150] flex items-center justify-center bg-black/50 p-6" onClick={() => setSettingsOpen(false)}>
          <div className="w-full max-w-3xl max-h-[90vh] overflow-y-auto rounded-xl border border-border bg-card p-6 shadow-xl" onClick={(e) => e.stopPropagation()}>
            <div className="mb-6 flex items-center justify-between">
              <h2 className="text-xl font-semibold text-foreground">{lang === 'pl' ? 'Ustawienia' : 'Settings'}</h2>
              <button type="button" onClick={() => setSettingsOpen(false)} className="rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground">
                <IconX className="h-5 w-5" />
              </button>
            </div>
            <div className="grid gap-6 md:grid-cols-2">
              <div className="space-y-6">
                <div>
                  <label className="mb-3 block text-sm font-medium text-foreground">
                    {lang === 'pl' ? 'Motyw' : 'Theme'}
                  </label>
                  <div className="flex flex-wrap gap-2">
                    {(['system', 'light', 'dark'] as const).map((t) => (
                      <button
                        key={t}
                        type="button"
                        onClick={() => setTheme(t)}
                        className={`rounded-lg border px-4 py-2 text-sm transition-colors ${
                          theme === t ? 'border-accent bg-accent text-accent-foreground' : 'border-border bg-background text-foreground hover:bg-muted'
                        }`}
                      >
                        {t === 'system' ? 'System' : t === 'light' ? (lang === 'pl' ? 'Jasny' : 'Light') : lang === 'pl' ? 'Ciemny' : 'Dark'}
                      </button>
                    ))}
                  </div>
                </div>
                <div>
                  <label className="mb-3 block text-sm font-medium text-foreground">
                    {lang === 'pl' ? 'Język' : 'Language'}
                  </label>
                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      onClick={() => setLang('pl')}
                      className={`rounded-lg border px-4 py-2 text-sm transition-colors ${
                        lang === 'pl'
                          ? 'border-accent bg-accent text-accent-foreground'
                          : 'border-border bg-background text-foreground hover:bg-muted'
                      }`}
                    >
                      Polski
                    </button>
                    <button
                      type="button"
                      onClick={() => setLang('en')}
                      className={`rounded-lg border px-4 py-2 text-sm transition-colors ${
                        lang === 'en'
                          ? 'border-accent bg-accent text-accent-foreground'
                          : 'border-border bg-background text-foreground hover:bg-muted'
                      }`}
                    >
                      English
                    </button>
                  </div>
                </div>
                <div className="flex items-center justify-between rounded-lg border border-border bg-background p-4">
                  <label className="text-sm font-medium text-foreground">
                    {lang === 'pl' ? 'Uruchamiaj przy starcie systemu' : 'Launch at system startup'}
                  </label>
                  <button
                    type="button"
                    onClick={async () => {
                      const v = !autostartEnabled
                      try {
                        if (v) await enable(); else await disable()
                        setAutostartEnabled(v)
                        await invoke('save_autostart_preference', { enabled: v })
                      } catch { /* ignore */ }
                    }}
                    className={`relative h-6 w-11 rounded-full transition-colors ${autostartEnabled ? 'bg-accent' : 'bg-muted'}`}
                  >
                    <div className={`absolute top-1 h-4 w-4 rounded-full bg-white transition-transform ${autostartEnabled ? 'translate-x-6' : 'translate-x-1'}`} />
                  </button>
                </div>
              </div>
              <div className="flex flex-col gap-4">
                <div>
                  <h3 className="mb-3 text-sm font-medium text-foreground">
                    {lang === 'pl' ? 'Rejestr zdarzeń' : 'Event log'}
                  </h3>
                  <div className="h-48 overflow-y-auto rounded-lg border border-border bg-background p-3">
                    <div className="space-y-2">
                      {log.map((line, i) => (
                        <div key={i} className="rounded border-l-2 border-accent bg-card px-3 py-2 font-mono text-xs">
                          <div className="text-foreground">{line}</div>
                        </div>
                      ))}
                    </div>
                  </div>
                </div>
                <div className="rounded-lg border border-border bg-background p-4">
                  <h4 className="mb-2 text-sm font-medium text-foreground">
                    {lang === 'pl' ? 'Komendy urządzenia (CDC)' : 'Device commands (CDC)'}
                  </h4>
                  <p className="mb-3 text-xs text-muted-foreground">
                    {lang === 'pl'
                      ? 'Wysyłane do połączonego portu szeregowego. Odpowiedzi pojawiają się powyżej jako linie zaczynające się od <code className="text-foreground/90">←</code>. Używaj, gdy urządzenie jest połączone.'
                      : 'Sent to the connected serial port. Replies appear above as lines starting with <code className="text-foreground/90">←</code>. Use when the device is connected.'}
                  </p>
                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      className="rounded-md border border-border bg-card px-2 py-1.5 text-xs text-foreground hover:bg-muted"
                      onClick={() => {
                        if (activePort) invoke('send_device_command', { portName: activePort, cmd: 'GET_STATE' }).catch(() => {})
                      }}
                    >
                      {lang === 'pl' ? 'POBIERZ_STAN' : 'GET_STATE'}
                    </button>
                    <button
                      type="button"
                      className="rounded-md border border-border bg-card px-2 py-1.5 text-xs text-foreground hover:bg-muted"
                      onClick={() => {
                        if (
                          window.confirm(
                            lang === 'pl'
                              ? 'Zresetować domyślne ustawienia urządzenia do wartości z firmware?'
                              : 'Reset device defaults to firmware values?'
                          )
                        ) {
                          if (activePort) invoke('send_device_command', { portName: activePort, cmd: 'RESET_DEFAULTS' }).catch(() => {})
                        }
                      }}
                    >
                      {lang === 'pl' ? 'RESETUJ_DOMYŚLNE' : 'RESET_DEFAULTS'}
                    </button>
                  </div>
                </div>
              </div>
            </div>
            <div className="mt-6 flex justify-end border-t border-border pt-6">
              <button type="button" onClick={() => setSettingsOpen(false)} className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-foreground transition-colors hover:bg-accent/90">
                {lang === 'pl' ? 'Zamknij' : 'Close'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

function targetLabel(t: VolumeTarget, lang: Lang): string {
  if (t.type === 'system') return lang === 'pl' ? 'Głośność systemu' : 'System volume'
  if (t.type === 'mic') return lang === 'pl' ? 'Głośność mikrofonu' : 'Microphone volume'
  if (t.type === 'category') {
    return t.id === 'gry' ? (lang === 'pl' ? 'Gry' : 'Games') : t.id
  }
  return t.name && t.name.trim() ? t.name : lang === 'pl' ? `Aplikacja ${t.pid}` : `App ${t.pid}`
}

interface ControllerCardProps {
  index: number
  sliderIndex: number
  showButtonMapping: boolean
  binding: ButtonBinding
  onBindingChange: (b: ButtonBinding) => void
  hwSliderMute: boolean
  onHwSliderMuteChange: (v: boolean) => void
  lang: Lang
  shortcutNeoOnShortcut: boolean
  onShortcutNeoChange: (v: boolean) => void
  value: number
  targets: VolumeTarget[]
  audioSessions: AudioSessionInfo[]
  loadAudioSessions: () => void
  onAssignmentsChange: (targets: VolumeTarget[]) => void
}

function bindingMode(b: ButtonBinding): 'none' | 'media' | 'shortcut' {
  if (b.kind === 'none') return 'none'
  if (b.kind === 'media') return 'media'
  return 'shortcut'
}

function ControllerCard({
  index,
  sliderIndex,
  showButtonMapping,
  binding,
  onBindingChange,
  hwSliderMute,
  onHwSliderMuteChange,
  lang,
  shortcutNeoOnShortcut,
  onShortcutNeoChange,
  value,
  targets,
  audioSessions,
  loadAudioSessions,
  onAssignmentsChange,
}: ControllerCardProps) {
  const [addOpen, setAddOpen] = useState(false)
  const [recordingShortcut, setRecordingShortcut] = useState(false)
  const captureRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (recordingShortcut) captureRef.current?.focus()
  }, [recordingShortcut])
  const [addSearch, setAddSearch] = useState('')
  const [actionOpen, setActionOpen] = useState(() => binding.kind !== 'none')
  const [assignmentsOpen, setAssignmentsOpen] = useState(false)

  useEffect(() => {
    if (binding.kind !== 'none') setActionOpen(true)
  }, [binding.kind])

  useEffect(() => {
    if (!assignmentsOpen) setAddOpen(false)
  }, [assignmentsOpen])

  const actionSummary =
    binding.kind === 'none'
      ? lang === 'pl'
        ? 'Brak'
        : 'None'
      : binding.kind === 'media'
        ? lang === 'pl'
          ? 'Klawisz multimediów'
          : 'Media key'
        : binding.label.trim()
          ? binding.label
          : lang === 'pl'
            ? 'Brak skrótu'
            : 'No shortcut'

  const normalizeAppName = (name?: string) =>
    (name || '').trim().toLowerCase().replace(/\.exe$/, '')
  const percent = Math.round((value / 1023) * 100)
  const activeSessions = audioSessions.filter((s) => s.is_active)
  const allFiltered = addSearch.trim()
    ? audioSessions.filter(
        (s) =>
          (s.name || '').toLowerCase().includes(addSearch.trim().toLowerCase()) ||
          String(s.pid).includes(addSearch.trim())
      )
    : audioSessions

  const removeTarget = (idx: number) => {
    const next = targets.filter((_, i) => i !== idx)
    onAssignmentsChange(next)
  }

  const addTarget = (t: VolumeTarget) => {
    const targetAppName = t.type === 'app' ? normalizeAppName(t.name) : ''
    if (
      targets.some(
        (x) =>
          x.type === t.type &&
          (x.type !== 'app' ||
            normalizeAppName((x as { name?: string }).name) === targetAppName ||
            x.pid === (t as { pid: number }).pid) &&
          (x.type !== 'category' || (x as { id: string }).id === (t as { id: string }).id)
      )
    )
      return
    onAssignmentsChange([...targets, t])
    setAddOpen(false)
  }

  return (
    <article className="rounded-xl border border-border bg-card p-6 shadow-sm transition-shadow hover:shadow-md flex flex-col relative">
      <div className="mb-4 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className="rounded-lg bg-muted p-2">
            <IconSliders className="h-5 w-5 text-foreground" />
          </div>
          <h3 className="font-medium text-foreground">Slider {index}</h3>
        </div>
        <div className="text-2xl font-bold text-foreground tabular-nums">{percent}%</div>
      </div>
      <div className="mb-6 h-2 overflow-hidden rounded-full bg-muted">
        <div className="h-full bg-accent transition-all duration-300" style={{ width: `${percent}%` }} />
      </div>
      {showButtonMapping && (
        <div className="mb-5 space-y-2 rounded-lg border border-border/70 bg-muted/30 p-3">
          <button
            type="button"
            onClick={() => setActionOpen((o) => !o)}
            className="w-full text-left"
          >
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              {lang === 'pl' ? 'Przycisk' : 'Button'} {sliderIndex}{' '}
              <span className="font-normal normal-case text-muted-foreground/80">
                {lang === 'pl' ? '(ten suwak)' : '(this slider)'}
              </span>
            </p>
            <p className="mt-1 text-xs text-muted-foreground">{actionSummary}</p>
          </button>
          <div className="flex flex-col gap-2">{actionOpen && (<> 
            <label className="text-xs text-muted-foreground">
              {lang === 'pl' ? 'Co wysłać w Windows' : 'What to send to Windows'}
            </label>
            <select
              value={bindingMode(binding)}
              onChange={(e) => {
                const v = e.target.value as 'none' | 'media' | 'shortcut'
                if (v === 'none') onBindingChange({ kind: 'none' })
                else if (v === 'media')
                  onBindingChange({
                    kind: 'media',
                    action: binding.kind === 'media' ? binding.action : 'play_pause',
                  })
                else
                  onBindingChange(
                    binding.kind === 'shortcut'
                      ? binding
                      : { kind: 'shortcut', vk: 0, mods: 0, label: '' },
                  )
              }}
              className="w-full rounded-lg border border-border bg-card px-2 py-1.5 text-sm text-foreground outline-none focus:border-accent"
            >
              <option value="none">{lang === 'pl' ? 'Nic' : 'None'}</option>
              <option value="media">{lang === 'pl' ? 'Klawisz multimediów' : 'Media key'}</option>
              <option value="shortcut">{lang === 'pl' ? 'Skrót klawiszowy' : 'Keyboard shortcut'}</option>
            </select>
            {bindingMode(binding) === 'media' && (
              <>
                <label className="text-xs text-muted-foreground">{lang === 'pl' ? 'Wybór' : 'Choice'}</label>
                <select
                  value={binding.kind === 'media' ? binding.action : 'play_pause'}
                  onChange={(e) =>
                    onBindingChange({ kind: 'media', action: e.target.value as MediaKeyAction })
                  }
                  className="w-full rounded-lg border border-border bg-card px-2 py-1.5 text-sm text-foreground outline-none focus:border-accent"
                >
                  {MEDIA_ACTION_OPTIONS.filter((o) => o.value !== 'none').map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </>
            )}
            {bindingMode(binding) === 'shortcut' && (
              <div className="space-y-2">
                <p className="text-xs text-muted-foreground">
                  {lang === 'pl' ? (
                    <>
                      Kliknij <strong className="text-foreground">Nagraj</strong>, potem naciśnij kombinację. Przy literach
                      i cyfrach użyj co najmniej <strong className="text-foreground">Ctrl, Shift, Alt lub Win</strong>{' '}
                      (F1–F12 i strzałki mogą być same). <kbd className="rounded bg-muted px-1">Esc</kbd> anuluje.
                    </>
                  ) : (
                    <>
                      Click <strong className="text-foreground">Record</strong>, then press the combination. For letters and
                      digits, use at least <strong className="text-foreground">Ctrl, Shift, Alt or Win</strong>{' '}
                      (F1–F12 and arrow keys can be alone). <kbd className="rounded bg-muted px-1">Esc</kbd> cancels.
                    </>
                  )}
                </p>
                <div className="rounded-md border border-border bg-card px-2 py-1.5 font-mono text-sm text-foreground">
                  {binding.kind === 'shortcut' && binding.label.trim() ? binding.label : '— brak —'}
                </div>
                <button
                  type="button"
                  onClick={() => setRecordingShortcut((r) => !r)}
                  className={`w-full rounded-lg border px-2 py-1.5 text-sm font-medium transition-colors ${
                    recordingShortcut
                      ? 'border-accent bg-accent/15 text-accent'
                      : 'border-border bg-background text-foreground hover:bg-muted'
                  }`}
                >
                  {recordingShortcut
                    ? lang === 'pl'
                      ? 'Nagrywanie… (Esc = stop)'
                      : 'Recording… (Esc = stop)'
                    : lang === 'pl'
                      ? 'Nagraj skrót'
                      : 'Record shortcut'}
                </button>
                {recordingShortcut && (
                  <div
                    ref={captureRef}
                    tabIndex={0}
                    role="textbox"
                    aria-label="Shortcut capture"
                    className="rounded-lg border-2 border-dashed border-accent bg-background/80 p-3 text-center text-xs text-muted-foreground outline-none ring-2 ring-accent/40"
                    onKeyDown={(e) => {
                      e.preventDefault()
                      e.stopPropagation()
                      if (e.repeat) return
                      if (e.code === 'Escape') {
                        setRecordingShortcut(false)
                        return
                      }
                      if (isModifierCode(e.code)) return
                      const vk = keyboardCodeToVk(e.code)
                      if (vk == null) return
                      const mods = modifierMask(e)
                      if (mods === 0 && !shortcutAllowedWithoutModifier(e.code)) return
                      const label = shortcutLabelFromEvent(e)
                      onBindingChange({ kind: 'shortcut', vk, mods, label })
                      setRecordingShortcut(false)
                    }}
                  >
                    {lang === 'pl' ? 'Naciśnij teraz skrót' : 'Press the shortcut now'}
                  </div>
                )}
              </div>
            )}
          </>)}
          <label className="mt-1 flex cursor-pointer items-center gap-2 text-xs text-muted-foreground">
              <input
                type="checkbox"
                className="rounded border-border"
                checked={hwSliderMute}
                onChange={(e) => onHwSliderMuteChange(e.target.checked)}
              />
              <span>{lang === 'pl' ? 'Wycisz suwak na urządzeniu' : 'Mute slider on device'}</span>
            </label>
            {!hwSliderMute && (
              <label className="mt-1 flex cursor-pointer items-center gap-2 text-xs text-muted-foreground">
                <input
                  type="checkbox"
                  className="rounded border-border"
                  checked={shortcutNeoOnShortcut}
                  onChange={(e) => onShortcutNeoChange(e.target.checked)}
                />
                <span>
                  {lang === 'pl'
                    ? 'Neo przy skrócie (LED jak przy wyciszeniu, gdy używasz skrótu zamiast mute na suwaku)'
                    : 'Neo on shortcut (LED like mute, when you use a keyboard shortcut instead of mute on the slider)'}
                </span>
              </label>
            )}
          </div>
        </div>
      )}
      <div className="space-y-3">
        <button
          type="button"
          className="w-full text-left"
          onClick={() => setAssignmentsOpen((o) => !o)}
        >
          <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            {lang === 'pl' ? 'Przypisania' : 'Assignments'} ({targets.length})
          </p>
        </button>
        {assignmentsOpen && (
          <div className="space-y-3">
          {/* v0 design: full-width rows per assignment */}
          {targets.map((t, i) => (
            <div
              key={
                t.type === 'app'
                  ? `app-${((t as { name?: string }).name || '').toLowerCase() || t.pid}`
                  : t.type === 'category'
                    ? `cat-${(t as { id: string }).id}`
                    : 'system'
              }
              className="flex items-center justify-between rounded-lg border border-border bg-background p-3"
            >
              <div className="flex items-center gap-2 min-w-0">
                <div
                  className={`h-2 w-2 shrink-0 rounded-full ${
                    t.type === 'system' ? 'bg-accent' : t.type === 'app' ? 'bg-blue-500' : 'bg-orange-500'
                  }`}
                />
                <span className="text-sm text-foreground truncate">{targetLabel(t, lang)}</span>
              </div>
              <button
                type="button"
                onClick={() => removeTarget(i)}
                className="shrink-0 text-muted-foreground hover:text-destructive focus:outline-none focus:ring-1 focus:ring-accent rounded p-0.5"
                aria-label={lang === 'pl' ? 'Usuń' : 'Remove'}
              >
                <IconX className="h-4 w-4" />
              </button>
            </div>
          ))}
          {/* v0: dashed "+ Add assignment" button */}
          <div className="relative">
            <button
              type="button"
              onClick={() => {
                setAddOpen((o) => !o)
                if (!addOpen) loadAudioSessions()
              }}
              className="w-full rounded-lg border border-dashed border-border bg-background px-3 py-2 text-sm text-muted-foreground transition-colors hover:border-accent hover:text-accent focus:outline-none focus:ring-2 focus:ring-accent focus:ring-offset-2 focus:ring-offset-card"
            >
              {lang === 'pl' ? '+ Dodaj przypisanie' : '+ Add assignment'}
            </button>
            {addOpen && (
              <>
                <div className="absolute z-[100] top-full left-0 right-0 mt-1.5 rounded-lg border border-border bg-card shadow-xl overflow-hidden flex flex-col w-full min-w-[240px] max-h-[min(480px,80vh)]">
                  {/* Górna część: System, Categories, Currently playing — stała wysokość, bez ściskania */}
                  <div className="p-2 flex flex-col gap-1 shrink-0">
                    <button
                      type="button"
                      className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 transition-colors"
                      onClick={() => addTarget({ type: 'system' })}
                    >
                      {lang === 'pl' ? 'Głośność systemu' : 'System volume'}
                    </button>
                    <button
                      type="button"
                      className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 transition-colors"
                      onClick={() => addTarget({ type: 'mic' })}
                    >
                      {lang === 'pl' ? 'Głośność mikrofonu' : 'Microphone volume'}
                    </button>
                    <div className="border-t border-border my-1" />
                    <p className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground px-3 pt-1 pb-0.5">
                      {lang === 'pl' ? 'Kategorie' : 'Categories'}
                    </p>
                    <button
                      type="button"
                      className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 flex items-center gap-2 transition-colors"
                      onClick={() => addTarget({ type: 'category', id: 'gry' })}
                    >
                      <span className="shrink-0">🎮</span> {lang === 'pl' ? 'Gry' : 'Games'}
                    </button>
                    <div className="border-t border-border my-1" />
                    <p className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground px-3 pt-1 pb-0.5">
                      {lang === 'pl' ? 'Aktualnie grane' : 'Currently playing'}
                    </p>
                    {activeSessions.length === 0 ? (
                      <p className="text-xs text-muted-foreground px-3 py-2">
                        {lang === 'pl' ? 'Brak aktywnych' : 'None active'}
                      </p>
                    ) : (
                      activeSessions.slice(0, 8).map((s) => (
                        <button type="button" key={s.pid} className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 truncate transition-colors" onClick={() => addTarget({ type: 'app', pid: s.pid, name: s.name })}>
                          {s.name || `PID ${s.pid}`}
                        </button>
                      ))
                    )}
                  </div>
                  {/* All applications: zajmuje resztę miejsca, lista ma własny scroll */}
                  <div className="border-t border-border flex flex-col flex-1 min-h-0 p-2">
                    <p className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground px-3 pt-0.5 pb-1">
                      {lang === 'pl' ? 'Wszystkie aplikacje' : 'All applications'}
                    </p>
                    <input
                      type="text"
                      placeholder={lang === 'pl' ? 'Szukaj…' : 'Search…'}
                      value={addSearch}
                      onChange={(e) => setAddSearch(e.target.value)}
                      className="rounded-md border border-border bg-background text-foreground text-sm px-3 py-2 w-full mb-2 focus:outline-none focus:ring-1 focus:ring-accent shrink-0"
                    />
                    <div className="flex-1 min-h-[140px] overflow-y-auto rounded-md border border-border/50 bg-background/50">
                      {allFiltered.length === 0 ? (
                        <p className="text-xs text-muted-foreground px-3 py-4 text-center">
                          {addSearch.trim() ? (lang === 'pl' ? 'Brak wyników' : 'No results') : (lang === 'pl' ? 'Ładowanie…' : 'Loading…')}
                        </p>
                      ) : (
                        <div className="py-0.5 space-y-0.5">
                          {allFiltered.map((s) => (
                            <button type="button" key={s.pid} className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 truncate flex items-center gap-2 transition-colors" onClick={() => addTarget({ type: 'app', pid: s.pid, name: s.name })}>
                              {s.is_game && <span className="shrink-0">🎮</span>}
                              {s.name || `PID ${s.pid}`}
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  </div>
                </div>
                <div className="fixed inset-0 z-[90]" aria-hidden onClick={() => setAddOpen(false)} />
              </>
            )}
          </div>
          </div>
        )}
      </div>
    </article>
  )
}

// Legacy: ControllerCard pozostaje w pliku jako historyczny komponent,
// ale obecnie UI używa `V0ControllerCard`. Dzięki temu TypeScript nie zgłasza noUnusedLocals.
void ControllerCard

export default App
