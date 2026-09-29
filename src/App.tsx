import { useEffect, useState, useCallback, useRef, useMemo } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { isEnabled, enable, disable } from '@tauri-apps/plugin-autostart'
import { TitleBar } from './TitleBar'
import { Settings, ChevronDown } from 'lucide-react'
import {
  ACCENT,
  ChannelDrawer,
  ChannelStrip,
  ConnectionPopover,
  ErrorReportingConsentModal,
  PresetsRail,
  SettingsSheet,
} from './console'
import type {
  AppUpdateInfo,
  Profile,
  ProfileData,
  ProfilesState,
  UpdateDownloadProgress,
} from './console'
import { reportError } from './errorReporting'
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
  exe_name?: string
  display_name?: string
  is_active?: boolean
  is_playing?: boolean
  is_game?: boolean
}

/** Maksymalna liczba suwaków w stanie / presetach — zgodna z `crate::audio::MAX_SLIDERS`. */
const MAX_SLIDERS = 5

/** Zgodne z `media_keys::MEDIA_BUTTON_SLOTS` i serde `snake_case`. */
const MEDIA_BUTTON_SLOTS = 5

type Lang = 'pl' | 'en'
const LANG_STORAGE_KEY = 'idei-lang'

type ShortcutLedMode = 'off' | 'momentary' | 'toggle'

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

function defaultHwSliderMute(): boolean[] {
  return Array.from({ length: MEDIA_BUTTON_SLOTS }, () => false)
}

function defaultShortcutLedModes(): ShortcutLedMode[] {
  return Array.from({ length: MEDIA_BUTTON_SLOTS }, () => 'off' as ShortcutLedMode)
}

function parseShortcutLedMode(x: unknown): ShortcutLedMode {
  if (x === 'momentary' || x === 'toggle' || x === 'off') return x
  if (x === true) return 'momentary'
  return 'off'
}

function emptySliderValues(): number[] {
  return Array.from({ length: MAX_SLIDERS }, () => 0)
}

function emptyAssignments(): VolumeTarget[][] {
  return Array.from({ length: MAX_SLIDERS }, () => [] as VolumeTarget[])
}

/** Legacy localStorage — migracja → backend profiles. */
type LegacyPresetEntry = {
  id: string
  name: string
  sliderValues?: number[]
  assignments?: VolumeTarget[][]
  buttonBindings?: ButtonBinding[]
  buttonMediaActions?: MediaKeyAction[]
  buttonHwSliderMute?: boolean[]
  shortcutLedMode?: ShortcutLedMode[]
  shortcutMuteLedMap?: boolean[]
  shortcutMuteLed?: boolean
}

const LEGACY_PRESETS_KEY = 'idei-presets'
const PRESETS_BY_MODEL_KEY = 'idei-presets-by-model'
const PROFILES_MIGRATED_KEY = 'idei-profiles-migrated-v1'

function legacyPresetToProfileData(p: LegacyPresetEntry): ProfileData {
  const assignments = emptyAssignments()
  for (let i = 0; i < MAX_SLIDERS; i++) {
    assignments[i] = Array.isArray(p.assignments?.[i]) ? [...(p.assignments as VolumeTarget[][])[i]] : []
  }
  let button_bindings = defaultButtonBindings()
  if (Array.isArray(p.buttonBindings) && p.buttonBindings.length === MEDIA_BUTTON_SLOTS) {
    const parsed = (p.buttonBindings as unknown[]).map(parseButtonBinding)
    if (parsed.every((b): b is ButtonBinding => b != null)) {
      button_bindings = parsed
    }
  } else if (Array.isArray(p.buttonMediaActions) && p.buttonMediaActions.length === MEDIA_BUTTON_SLOTS) {
    button_bindings = (p.buttonMediaActions as MediaKeyAction[]).map(mediaRowToBinding)
  }
  let button_hw_slider_mute = defaultHwSliderMute()
  if (Array.isArray(p.buttonHwSliderMute) && p.buttonHwSliderMute.length === MEDIA_BUTTON_SLOTS) {
    button_hw_slider_mute = [...p.buttonHwSliderMute]
  }
  let shortcut_led_mode = defaultShortcutLedModes()
  if (Array.isArray(p.shortcutLedMode) && p.shortcutLedMode.length === MEDIA_BUTTON_SLOTS) {
    shortcut_led_mode = p.shortcutLedMode.map(parseShortcutLedMode)
  } else if (Array.isArray(p.shortcutMuteLedMap) && p.shortcutMuteLedMap.length === MEDIA_BUTTON_SLOTS) {
    shortcut_led_mode = p.shortcutMuteLedMap.map((b) => (b ? 'momentary' : 'off') as ShortcutLedMode)
  } else if (p.shortcutMuteLed === true) {
    shortcut_led_mode = Array.from({ length: MEDIA_BUTTON_SLOTS }, () => 'momentary' as ShortcutLedMode)
  }
  return { volume_assignments: assignments, button_bindings, button_hw_slider_mute, shortcut_led_mode }
}

function collectLegacyPresets(): LegacyPresetEntry[] {
  const out: LegacyPresetEntry[] = []
  try {
    const raw = localStorage.getItem(PRESETS_BY_MODEL_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as Record<string, unknown>
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        for (const v of Object.values(parsed)) {
          if (Array.isArray(v)) out.push(...(v as LegacyPresetEntry[]))
        }
      }
    }
  } catch {
    /* ignore */
  }
  if (out.length === 0) {
    try {
      const raw = localStorage.getItem(LEGACY_PRESETS_KEY)
      if (raw) {
        const parsed = JSON.parse(raw) as LegacyPresetEntry[]
        if (Array.isArray(parsed)) out.push(...parsed)
      }
    } catch {
      /* ignore */
    }
  }
  return out
}

function clearLegacyPresetStorage() {
  try {
    localStorage.removeItem(LEGACY_PRESETS_KEY)
    localStorage.removeItem(PRESETS_BY_MODEL_KEY)
    localStorage.setItem(PROFILES_MIGRATED_KEY, '1')
  } catch {
    /* ignore */
  }
}

function profileTargetsConflict(a: VolumeTarget, b: VolumeTarget): boolean {
  if (a.type === 'system' && b.type === 'system') return true
  if (a.type === 'mic' && b.type === 'mic') return true
  if (a.type === 'app' && b.type === 'app') {
    if (a.pid === b.pid) return true
    const na = (a.name ?? '').trim().toLowerCase().replace(/\.exe$/i, '')
    const nb = (b.name ?? '').trim().toLowerCase().replace(/\.exe$/i, '')
    return na.length > 0 && na === nb
  }
  if (a.type === 'category' && b.type === 'category') return a.id === b.id
  return false
}

function profileDataFromUi(
  assignments: VolumeTarget[][],
  buttonBindings: ButtonBinding[],
  buttonHwSliderMute: boolean[],
  shortcutLedMode: ShortcutLedMode[],
): ProfileData {
  return {
    volume_assignments: assignments.map((a) => [...a]),
    button_bindings: [...buttonBindings],
    button_hw_slider_mute: [...buttonHwSliderMute],
    shortcut_led_mode: [...shortcutLedMode],
  }
}

function applyProfileDataToUiState(
  data: ProfileData,
  setters: {
    setAssignments: (a: VolumeTarget[][]) => void
    setButtonBindings: (b: ButtonBinding[]) => void
    setButtonHwSliderMute: (b: boolean[]) => void
    setShortcutLedMode: (m: ShortcutLedMode[]) => void
  },
  targetsConflict: (a: VolumeTarget, b: VolumeTarget) => boolean,
) {
  const sanitized: VolumeTarget[][] = Array.from({ length: MAX_SLIDERS }, () => [])
  for (let i = 0; i < MAX_SLIDERS; i++) {
    const kept: VolumeTarget[] = []
    for (const t of data.volume_assignments[i] ?? []) {
      let conflict = false
      for (let j = 0; j < i && !conflict; j++) {
        for (const u of sanitized[j]) {
          if (targetsConflict(t, u)) {
            conflict = true
            break
          }
        }
      }
      if (!conflict) kept.push(t)
    }
    sanitized[i] = kept
  }
  setters.setAssignments(sanitized)
  setters.setButtonBindings(
    data.button_bindings?.length === MEDIA_BUTTON_SLOTS
      ? [...data.button_bindings]
      : defaultButtonBindings(),
  )
  setters.setButtonHwSliderMute(
    data.button_hw_slider_mute?.length === MEDIA_BUTTON_SLOTS
      ? [...data.button_hw_slider_mute]
      : defaultHwSliderMute(),
  )
  setters.setShortcutLedMode(
    data.shortcut_led_mode?.length === MEDIA_BUTTON_SLOTS
      ? data.shortcut_led_mode.map(parseShortcutLedMode)
      : defaultShortcutLedModes(),
  )
  return sanitized
}

/** Zamienia surowe komunikaty błędów na krótkie, zrozumiałe dla użytkownika. */
function formatConnectionError(raw: string): string {
  const s = raw.toLowerCase()
  if (s.includes('timeout') || s.includes('timed out')) return 'Connection timed out — check the USB cable and try again.'
  if (s.includes('access denied') || s.includes('odmowa dostępu') || s.includes('permission')) return 'Access denied to HID device — close other apps using it or check permissions.'
  if (s.includes('in use') || s.includes('używan') || s.includes('already open') || s.includes('busy')) return 'Device is in use by another application.'
  if (s.includes('not found') || s.includes('nie znaleziono') || s.includes('no device')) return 'Device not found — check the USB cable and try Scan again.'
  if (s.includes('could not open') || s.includes('failed to open') || s.includes('nie można otworzyć')) return 'Could not open HID device — check the USB cable.'
  if (s.includes('get info')) return 'Device did not answer Get Info — check firmware / HID interface.'
  return raw || 'Connection error. Check the USB cable.'
}

function shortDeviceLabel(path: string, info?: DeviceInfo | null): string {
  if (info?.model) return info.model
  if (!path) return '—'
  const tail = path.split(/[\\/#]/).filter(Boolean).pop() ?? path
  return tail.length > 24 ? `…${tail.slice(-24)}` : tail
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
  const [errorReportingConsent, setErrorReportingConsent] = useState<boolean | null>(null)
  const [errorConsentReady, setErrorConsentReady] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [appVersion, setAppVersion] = useState('0.1.0')
  const [appUpdateInfo, setAppUpdateInfo] = useState<AppUpdateInfo | null>(null)
  const [appUpdateChecking, setAppUpdateChecking] = useState(false)
  const [appUpdateInstalling, setAppUpdateInstalling] = useState(false)
  const [appUpdateError, setAppUpdateError] = useState<string | null>(null)
  const [downloadProgress, setDownloadProgress] = useState<UpdateDownloadProgress | null>(null)
  const [showConn, setShowConn] = useState(false)
  const [openDrawer, setOpenDrawer] = useState<number | null>(null)
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
  const [profiles, setProfiles] = useState<Profile[]>([])
  const [activeProfileId, setActiveProfileId] = useState<string | null>(null)
  const [profileSavedHint, setProfileSavedHint] = useState(false)
  const [shortcutLedMode, setShortcutLedMode] = useState<ShortcutLedMode[]>(() => defaultShortcutLedModes())
  const [shortcutLedLatched, setShortcutLedLatched] = useState<boolean[]>(() =>
    Array.from({ length: MEDIA_BUTTON_SLOTS }, () => false),
  )
  const [buttonBindings, setButtonBindings] = useState<ButtonBinding[]>(() => defaultButtonBindings())
  const [buttonHwSliderMute, setButtonHwSliderMute] = useState<boolean[]>(() => defaultHwSliderMute())
  const rawValuesRef = useRef<number[]>(emptySliderValues())
  const profileSaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const profileSavedHintTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const profilesReadyRef = useRef(false)
  const perPortStateRef = useRef<
    Record<
      string,
      {
        sliderValues: number[]
        assignments: VolumeTarget[][]
        buttonBindings: ButtonBinding[]
        buttonHwSliderMute: boolean[]
        shortcutLedMode: ShortcutLedMode[]
      }
    >
  >({})
  const perPortAssignmentsReadyRef = useRef<Record<string, boolean>>({})
  const device = useMemo(() => (activePort ? connectedDevices[activePort] ?? null : null), [connectedDevices, activePort])

  const sortDevicePaths = (paths: string[]): string[] => {
    return [...paths].sort((a, b) => a.localeCompare(b))
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

  const applyProfilesState = useCallback((state: ProfilesState) => {
    setProfiles(state.profiles ?? [])
    setActiveProfileId(state.active_profile_id ?? null)
  }, [])

  const showSavedHint = useCallback(() => {
    setProfileSavedHint(true)
    if (profileSavedHintTimerRef.current) clearTimeout(profileSavedHintTimerRef.current)
    profileSavedHintTimerRef.current = setTimeout(() => setProfileSavedHint(false), 1200)
  }, [])

  const syncUiFromActiveProfile = useCallback((state: ProfilesState) => {
    const active = state.profiles.find((p) => p.id === state.active_profile_id)
    if (!active) return
    applyProfileDataToUiState(
      active,
      { setAssignments, setButtonBindings, setButtonHwSliderMute, setShortcutLedMode },
      profileTargetsConflict,
    )
  }, [])

  const assignmentsRef = useRef(assignments)
  const buttonBindingsRef = useRef(buttonBindings)
  const buttonHwSliderMuteRef = useRef(buttonHwSliderMute)
  const shortcutLedModeRef = useRef(shortcutLedMode)
  assignmentsRef.current = assignments
  buttonBindingsRef.current = buttonBindings
  buttonHwSliderMuteRef.current = buttonHwSliderMute
  shortcutLedModeRef.current = shortcutLedMode

  const scheduleSaveCurrentProfile = useCallback(() => {
    if (!profilesReadyRef.current || !activeProfileId) return
    if (profileSaveTimerRef.current) clearTimeout(profileSaveTimerRef.current)
    profileSaveTimerRef.current = setTimeout(() => {
      const data = profileDataFromUi(
        assignmentsRef.current,
        buttonBindingsRef.current,
        buttonHwSliderMuteRef.current,
        shortcutLedModeRef.current,
      )
      invoke<ProfilesState>('save_current_profile', { profileData: data })
        .then((state) => {
          applyProfilesState(state)
          showSavedHint()
        })
        .catch(() => {})
    }, 450)
  }, [activeProfileId, applyProfilesState, showSavedHint])

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        let state = await invoke<ProfilesState>('get_profiles')
        if (cancelled) return

        let migratedFlag = false
        try {
          migratedFlag = localStorage.getItem(PROFILES_MIGRATED_KEY) === '1'
        } catch {
          migratedFlag = false
        }

        if (!migratedFlag) {
          const legacy = collectLegacyPresets()
          if (legacy.length > 0) {
            const onlyDefault =
              state.profiles.length === 1 &&
              (state.profiles[0]?.is_default || state.profiles[0]?.name === 'Default')
            if (onlyDefault || state.profiles.length === 0) {
              for (const p of legacy) {
                const name = (p.name && p.name.trim()) || 'Imported'
                state = await invoke<ProfilesState>('create_profile', {
                  name,
                  baseProfileId: null,
                  seed: legacyPresetToProfileData(p),
                })
              }
            }
            clearLegacyPresetStorage()
          } else {
            clearLegacyPresetStorage()
          }
        }

        if (cancelled) return
        applyProfilesState(state)
        syncUiFromActiveProfile(state)
        profilesReadyRef.current = true
      } catch {
        profilesReadyRef.current = true
      }
    })()
    return () => {
      cancelled = true
      if (profileSaveTimerRef.current) clearTimeout(profileSaveTimerRef.current)
      if (profileSavedHintTimerRef.current) clearTimeout(profileSavedHintTimerRef.current)
    }
    // Jednorazowo przy starcie.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const selectProfile = useCallback(
    async (id: string) => {
      if (!id || id === activeProfileId) return
      try {
        const state = await invoke<ProfilesState>('set_active_profile', { profileId: id })
        applyProfilesState(state)
        syncUiFromActiveProfile(state)
        showSavedHint()
      } catch {
        /* ignore */
      }
    },
    [activeProfileId, applyProfilesState, syncUiFromActiveProfile, showSavedHint],
  )

  const addProfile = useCallback(
    async (name?: string) => {
      try {
        const state = await invoke<ProfilesState>('create_profile', {
          name: (name && name.trim()) || 'New profile',
          baseProfileId: null,
          seed: profileDataFromUi(assignments, buttonBindings, buttonHwSliderMute, shortcutLedMode),
        })
        applyProfilesState(state)
        syncUiFromActiveProfile(state)
        showSavedHint()
      } catch {
        /* ignore */
      }
    },
    [
      assignments,
      buttonBindings,
      buttonHwSliderMute,
      shortcutLedMode,
      applyProfilesState,
      syncUiFromActiveProfile,
      showSavedHint,
    ],
  )

  const duplicateProfile = useCallback(
    async (id: string) => {
      const base = profiles.find((p) => p.id === id)
      if (!base) return
      try {
        const state = await invoke<ProfilesState>('create_profile', {
          name: `${base.name} copy`,
          baseProfileId: id,
          seed: null,
        })
        applyProfilesState(state)
        syncUiFromActiveProfile(state)
        showSavedHint()
      } catch {
        /* ignore */
      }
    },
    [profiles, applyProfilesState, syncUiFromActiveProfile, showSavedHint],
  )

  const renameProfile = useCallback(
    async (id: string, name: string) => {
      try {
        const state = await invoke<ProfilesState>('rename_profile', {
          profileId: id,
          newName: name.trim() || 'Profile',
        })
        applyProfilesState(state)
      } catch {
        /* ignore */
      }
    },
    [applyProfilesState],
  )

  const deleteProfile = useCallback(
    async (id: string) => {
      try {
        const state = await invoke<ProfilesState>('delete_profile', { profileId: id })
        applyProfilesState(state)
        syncUiFromActiveProfile(state)
        showSavedHint()
      } catch {
        /* ignore */
      }
    },
    [applyProfilesState, syncUiFromActiveProfile, showSavedHint],
  )

  const runScan = useCallback(() => {
    setScanning(true)
    invoke('start_scan_idei_devices').catch(() => setScanning(false))
  }, [])

  const saveActivePortState = useCallback(() => {
    if (!activePort) return
    perPortStateRef.current[activePort] = {
      sliderValues: [...sliderValues],
      assignments: assignments.map((a) => [...a]),
      buttonBindings: [...buttonBindings],
      buttonHwSliderMute: [...buttonHwSliderMute],
      shortcutLedMode: [...shortcutLedMode],
    }
  }, [activePort, sliderValues, assignments, buttonBindings, buttonHwSliderMute, shortcutLedMode])

  const loadPortState = useCallback((port: string, applyToUi = false) => {
    const local = perPortStateRef.current[port]
    if (local && applyToUi) {
      setSliderValues(local.sliderValues)
      setAssignments(local.assignments)
      setButtonBindings(local.buttonBindings)
      setButtonHwSliderMute(local.buttonHwSliderMute)
      setShortcutLedMode(local.shortcutLedMode)
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
          shortcutLedMode: cur?.shortcutLedMode ?? defaultShortcutLedModes(),
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
          shortcutLedMode: cur?.shortcutLedMode ?? defaultShortcutLedModes(),
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
            shortcutLedMode: cur?.shortcutLedMode ?? defaultShortcutLedModes(),
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
          shortcutLedMode: cur?.shortcutLedMode ?? defaultShortcutLedModes(),
        }
        if (applyToUi) setButtonHwSliderMute(next)
      }
    }).catch(() => {})
    invoke('get_shortcut_led_mode', { portName: port }).then((list: unknown) => {
      if (Array.isArray(list) && list.length === MEDIA_BUTTON_SLOTS) {
        const next = list.map(parseShortcutLedMode)
        const cur = perPortStateRef.current[port]
        perPortStateRef.current[port] = {
          sliderValues: cur?.sliderValues ?? emptySliderValues(),
          assignments: cur?.assignments ?? emptyAssignments(),
          buttonBindings: cur?.buttonBindings ?? defaultButtonBindings(),
          buttonHwSliderMute: cur?.buttonHwSliderMute ?? defaultHwSliderMute(),
          shortcutLedMode: next,
        }
        if (applyToUi) setShortcutLedMode(next)
      }
    }).catch(() => {})
    invoke('get_shortcut_led_latched', { portName: port }).then((list: unknown) => {
      if (applyToUi && Array.isArray(list) && list.length === MEDIA_BUTTON_SLOTS) {
        setShortcutLedLatched(list.map((x) => !!x))
      }
    }).catch(() => {})
  }, [])

  const connectToPort = useCallback(async (portName: string) => {
    setConnecting(true)
    setError(null)
    try {
      await invoke('connect_device', { path: portName })
    } catch (e) {
      setError(formatConnectionError(String(e)))
      setConnecting(false)
    }
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
      await invoke('disconnect_device', { path: targetPort })
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

  useEffect(() => {
    invoke<string>('get_app_version')
      .then(setAppVersion)
      .catch(() => {})
  }, [])

  useEffect(() => {
    let unlisten: (() => void) | undefined
    listen<UpdateDownloadProgress>('update-download-progress', (e) => {
      setDownloadProgress(e.payload)
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [])

  // Stan autostartu (plugin)
  useEffect(() => {
    isEnabled().then(setAutostartEnabled).catch(() => {})
  }, [])

  // Zgoda na raportowanie błędów (None → modal onboardingowy)
  useEffect(() => {
    invoke<{ error_reporting_consent?: boolean | null }>('get_app_config')
      .then((cfg) => {
        const v = cfg?.error_reporting_consent
        setErrorReportingConsent(typeof v === 'boolean' ? v : null)
      })
      .catch(() => setErrorReportingConsent(null))
      .finally(() => setErrorConsentReady(true))
  }, [])

  const applyErrorReportingConsent = useCallback(async (enabled: boolean) => {
    try {
      await invoke('set_error_reporting_consent', { enabled })
      setErrorReportingConsent(enabled)
    } catch {
      /* ignore */
    }
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
  const [systemDark, setSystemDark] = useState(
    () => typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches,
  )
  useEffect(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)')
    const onChange = () => setSystemDark(mq.matches)
    mq.addEventListener('change', onChange)
    return () => mq.removeEventListener('change', onChange)
  }, [])
  const isDark = theme === 'dark' || (theme === 'system' && systemDark)
  useEffect(() => {
    localStorage.setItem('idei-theme', theme)
  }, [theme])

  // Splash: ukryj po 1.5 s
  useEffect(() => {
    const t = setTimeout(() => setSplashVisible(false), 1500)
    return () => clearTimeout(t)
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
        shortcutLedMode: [...shortcutLedMode],
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
      const merged = sortDevicePaths(Array.from(new Set([...list, ...connected])))
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
      setConnectedDevices((prev) => ({ ...prev, [d.port]: d }))
      setSelectedPort(d.port)
      setActivePort(d.port)
      perPortAssignmentsReadyRef.current[d.port] = false
      setIdeiPorts((prev) => sortDevicePaths(Array.from(new Set([...prev, d.port]))))
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
      const raw = String(e.payload ?? 'Connection error')
      void reportError('HID connect error', { kind: 'hid', message: raw })
      setError(formatConnectionError(e.payload ?? 'Connection error'))
      setConnecting(false)
    })
    const unlistenDisconnect = listen<string>('device-disconnected-port', async (e) => {
      const port = e.payload
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
        const nextPort = sortDevicePaths(keys)[0]
        return nextPort ?? null
      })
      if (activePort === port) {
        const z = emptySliderValues()
        setSliderValues(z)
        rawValuesRef.current = [...z]
        setShortcutLedLatched(Array.from({ length: MEDIA_BUTTON_SLOTS }, () => false))
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
        shortcutLedMode: cur?.shortcutLedMode ?? defaultShortcutLedModes(),
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
    const unlistenLedLatched = listen<{ port: string; states: boolean[] }>('shortcut-led-latched-port', (e) => {
      const p = e.payload?.port
      const states = e.payload?.states
      if (!p || !Array.isArray(states)) return
      if (activePort === p) {
        setShortcutLedLatched(
          Array.from({ length: MEDIA_BUTTON_SLOTS }, (_, i) => !!states[i]),
        )
      }
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
      unlistenLedLatched.then((u) => u())
    }
  }, [connectToPort, loadPortState, activePort, connectedDevices])

  const isSelectedConnected = !!(selectedPort && connectedDevices[selectedPort])

  const sliderCount = device?.sliders ?? 0

  useEffect(() => {
    setOpenDrawer(null)
  }, [activePort, sliderCount])

  return (
    <div className={`app-container console-ui flex flex-col h-full min-h-full overflow-hidden rounded-2xl ${isDark ? 'dark' : 'light'}`}>
      {splashVisible && (
        <div className="fixed inset-0 z-[200] flex items-center justify-center rounded-2xl" style={{ background: 'var(--c-bg)' }}>
          <div className="flex flex-col items-center gap-4">
            <div className="w-16 h-16 rounded-2xl flex items-center justify-center" style={{ background: 'color-mix(in srgb, var(--c-accent) 20%, transparent)' }}>
              <img src="/assets/logo.png" alt="" />
            </div>
            <span style={{ fontSize: 16, color: 'var(--c-text)' }}>IDEI Control</span>
            <span style={{ fontSize: 13, color: 'var(--c-text-dim)' }}>Loading…</span>
          </div>
        </div>
      )}

      <TitleBar />
      <div className="flex-1 min-h-0 overflow-auto" style={{ background: 'var(--c-bg)', padding: 24, color: 'var(--c-text)' }}>
        <div style={{ maxWidth: 1200, margin: '0 auto' }}>
          <div
            style={{
              display: 'flex',
              justifyContent: 'space-between',
              alignItems: 'center',
              paddingBottom: 16,
              borderBottom: '0.5px solid var(--c-border)',
              marginBottom: 20,
              position: 'relative',
            }}
          >
            <div
              onClick={() => setShowConn((v) => !v)}
              style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13, color: 'var(--c-text-muted)', cursor: 'pointer' }}
            >
              <span
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: '50%',
                  background: device ? ACCENT : 'var(--c-text-faint)',
                  display: 'inline-block',
                  boxShadow: device ? `0 0 8px ${ACCENT}` : 'none',
                }}
              />
              <span style={{ color: 'var(--c-text)' }}>{device ? device.model : lang === 'pl' ? 'Brak urządzenia' : 'No device'}</span>
              <span style={{ color: 'var(--c-text-dim)' }}>
                · {device ? shortDeviceLabel(device.port, device) : selectedPort ? shortDeviceLabel(selectedPort) : '—'}
              </span>
              <ChevronDown
                size={13}
                color="var(--c-text-dim)"
                style={{ transform: showConn ? 'rotate(180deg)' : undefined, transition: 'transform 0.15s' }}
              />
            </div>
            {showConn && (
              <ConnectionPopover
                lang={lang}
                devices={ideiPorts}
                selectedPath={selectedPort}
                connectedDevices={connectedDevices}
                scanning={scanning}
                connecting={connecting}
                isSelectedConnected={isSelectedConnected}
                canDisconnect={!!activePort}
                onSelectDevice={(path) => {
                  setSelectedPort(path)
                  if (connectedDevices[path]) setActivePort(path)
                }}
                onConnect={handleConnect}
                onDisconnect={() => {
                  void handleDisconnect()
                  setShowConn(false)
                }}
                onScan={runScan}
              />
            )}
            <div style={{ display: 'flex', alignItems: 'center', gap: 14, fontSize: 12, color: 'var(--c-text-dim)' }}>
              {error && (
                <span style={{ color: 'var(--c-danger)', maxWidth: 280, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} role="alert">
                  {error}
                </span>
              )}
              {device?.fw ? <span>fw {device.fw}</span> : null}
              <button
                type="button"
                onClick={() => setSettingsOpen(true)}
                aria-label="Settings"
                style={{ background: 'transparent', border: 'none', color: 'var(--c-text-muted)', cursor: 'pointer', display: 'flex' }}
              >
                <Settings size={16} />
              </button>
            </div>
          </div>

          <div style={{ display: 'flex', gap: 16, alignItems: 'flex-start' }}>
            <div
              style={{
                flex: 1,
                display: 'grid',
                gridTemplateColumns: `repeat(${Math.max(sliderCount, 1)}, minmax(0, 1fr))`,
                gap: 14,
                alignContent: 'start',
              }}
            >
              {sliderCount > 0 ? (
                Array.from({ length: sliderCount }, (_, i) => (
                  <ChannelStrip
                    key={i}
                    index={i}
                    value={sliderValues[i] ?? 0}
                    targets={assignments[i] ?? []}
                    binding={buttonBindings[i] ?? { kind: 'none' }}
                    isOpen={openDrawer === i}
                    lang={lang}
                    allAssignments={assignments}
                    onToggle={() => setOpenDrawer(openDrawer === i ? null : i)}
                  />
                ))
              ) : (
                <div
                  style={{
                    gridColumn: '1 / -1',
                    background: 'var(--c-card)',
                    border: '0.5px solid var(--c-border)',
                    borderRadius: 10,
                    padding: 28,
                    color: 'var(--c-text-dim)',
                    fontSize: 13,
                    textAlign: 'center',
                  }}
                >
                  {lang === 'pl'
                    ? 'Podłącz urządzenie ideiMx, aby zobaczyć suwaki.'
                    : 'Connect an ideiMx device to see sliders.'}
                </div>
              )}
              {openDrawer !== null && sliderCount > 0 && openDrawer < sliderCount && (
                <ChannelDrawer
                  index={openDrawer}
                  lang={lang}
                  showButtonMapping={openDrawer < MEDIA_BUTTON_SLOTS}
                  targets={assignments[openDrawer] ?? []}
                  binding={buttonBindings[openDrawer] ?? { kind: 'none' }}
                  hwSliderMute={!!buttonHwSliderMute[openDrawer]}
                  shortcutLedMode={shortcutLedMode[openDrawer] ?? 'off'}
                  shortcutLedLatched={!!shortcutLedLatched[openDrawer]}
                  audioSessions={audioSessions}
                  allAssignments={assignments}
                  assignmentError={assignmentErrorBySlider[openDrawer] ?? null}
                  loadAudioSessions={loadAudioSessions}
                  onClose={() => setOpenDrawer(null)}
                  onBindingChange={(b) => {
                    const next = [...buttonBindings]
                    while (next.length < MEDIA_BUTTON_SLOTS) next.push({ kind: 'none' })
                    next[openDrawer] = b
                    buttonBindingsRef.current = next
                    setButtonBindings(next)
                    if (activePort) invoke('set_button_bindings', { portName: activePort, bindings: next }).catch(() => {})
                    scheduleSaveCurrentProfile()
                  }}
                  onHwSliderMuteChange={(v) => {
                    const next = [...buttonHwSliderMute]
                    while (next.length < MEDIA_BUTTON_SLOTS) next.push(false)
                    next[openDrawer] = v
                    buttonHwSliderMuteRef.current = next
                    setButtonHwSliderMute(next)
                    if (activePort) invoke('set_button_hw_slider_mute', { portName: activePort, enabled: next }).catch(() => {})
                    scheduleSaveCurrentProfile()
                  }}
                  onShortcutLedModeChange={(mode) => {
                    const next = [...shortcutLedMode]
                    while (next.length < MEDIA_BUTTON_SLOTS) next.push('off')
                    next[openDrawer] = mode
                    shortcutLedModeRef.current = next
                    setShortcutLedMode(next)
                    if (mode !== 'toggle') {
                      setShortcutLedLatched((prev) => {
                        const n = [...prev]
                        while (n.length < MEDIA_BUTTON_SLOTS) n.push(false)
                        n[openDrawer] = false
                        return n
                      })
                    }
                    if (activePort) invoke('set_shortcut_led_mode', { portName: activePort, modes: next }).catch(() => {})
                    scheduleSaveCurrentProfile()
                  }}
                  onAssignmentsChange={(targets) => {
                    const i = openDrawer
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
                      assignmentsRef.current = next.map((a) => [...a])
                      if (activePort) {
                        const cur = perPortStateRef.current[activePort]
                        perPortStateRef.current[activePort] = {
                          sliderValues: cur?.sliderValues ?? [...sliderValues],
                          assignments: next.map((a) => [...a]),
                          buttonBindings: cur?.buttonBindings ?? [...buttonBindings],
                          buttonHwSliderMute: cur?.buttonHwSliderMute ?? [...buttonHwSliderMute],
                          shortcutLedMode: cur?.shortcutLedMode ?? [...shortcutLedMode],
                        }
                      }
                      return next
                    })
                    if (activePort) invoke('set_volume_assignment', { portName: activePort, sliderIndex: i, targets }).catch(() => {})
                    scheduleSaveCurrentProfile()
                    return true
                  }}
                />
              )}
            </div>
            <PresetsRail
              modelLabel={device?.model ?? '—'}
              lang={lang}
              profiles={profiles}
              activeId={activeProfileId}
              savedHint={profileSavedHint}
              onSelect={selectProfile}
              onRename={renameProfile}
              onDuplicate={duplicateProfile}
              onDelete={deleteProfile}
              onAdd={addProfile}
            />
          </div>
        </div>
      </div>

      {settingsOpen && (
        <SettingsSheet
          lang={lang}
          theme={theme}
          autostartEnabled={autostartEnabled}
          log={log}
          canSendCommands={!!activePort}
          appVersion={appVersion}
          appUpdateInfo={appUpdateInfo}
          appUpdateChecking={appUpdateChecking}
          appUpdateInstalling={appUpdateInstalling}
          appUpdateError={appUpdateError}
          downloadProgress={downloadProgress}
          deviceFw={device?.fw ?? null}
          errorReportingEnabled={errorReportingConsent === true}
          onThemeChange={setTheme}
          onLangChange={setLang}
          onAutostartChange={async (v) => {
            try {
              if (v) await enable()
              else await disable()
              setAutostartEnabled(v)
              await invoke('save_autostart_preference', { enabled: v })
            } catch {
              /* ignore */
            }
          }}
          onErrorReportingChange={(v) => {
            void applyErrorReportingConsent(v)
          }}
          onGetState={() => {
            if (activePort) invoke('send_device_command', { portName: activePort, cmd: 'GET_STATE' }).catch(() => {})
          }}
          onResetDefaults={() => {
            if (activePort) invoke('send_device_command', { portName: activePort, cmd: 'RESET_DEFAULTS' }).catch(() => {})
          }}
          onCheckAppUpdate={async () => {
            setAppUpdateError(null)
            setAppUpdateChecking(true)
            try {
              const info = await invoke<AppUpdateInfo>('check_app_update')
              setAppUpdateInfo(info)
            } catch (e) {
              setAppUpdateError(String(e))
            } finally {
              setAppUpdateChecking(false)
            }
          }}
          onInstallAppUpdate={async () => {
            setAppUpdateError(null)
            setAppUpdateInstalling(true)
            setDownloadProgress(null)
            try {
              await invoke('install_app_update')
            } catch (e) {
              setAppUpdateError(String(e))
              setAppUpdateInstalling(false)
            }
          }}
          onClose={() => setSettingsOpen(false)}
        />
      )}

      {errorConsentReady && errorReportingConsent === null && (
        <ErrorReportingConsentModal
          lang={lang}
          onAllow={() => {
            void applyErrorReportingConsent(true)
          }}
          onDecline={() => {
            void applyErrorReportingConsent(false)
          }}
        />
      )}

      {showConn && (
        <div className="fixed inset-0 z-[15]" aria-hidden onClick={() => setShowConn(false)} />
      )}
    </div>
  )
}

export default App
