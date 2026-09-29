import { useEffect, useMemo, useRef, useState } from 'react'
import { Gamepad2, Mic, Monitor, Search, Sliders, VolumeX, Radio, X } from 'lucide-react'
import { AppSessionCombobox } from './AppSessionCombobox'
import {
  ACCENT,
  AssignmentRow,
  Chip,
  MEDIA_KEYS,
  Toggle,
  targetDisplayName,
} from './shared'
import type { AudioSessionInfo, ButtonBinding, Lang, MediaKeyAction, ShortcutLedMode, VolumeTarget } from './types'

const MOD_CTRL = 1
const MOD_SHIFT = 2
const MOD_ALT = 4
const MOD_WIN = 8

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
  for (let i = 0; i <= 9; i++) map[`Digit${i}`] = 0x30 + i
  for (let i = 1; i <= 12; i++) map[`F${i}`] = 0x6f + i
  for (let c = 65; c <= 90; c++) map[`Key${String.fromCharCode(c)}`] = c
  for (let i = 0; i <= 9; i++) map[`Numpad${i}`] = 0x60 + i
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

function bindingMode(b: ButtonBinding): 'none' | 'media' | 'shortcut' {
  if (b.kind === 'none') return 'none'
  if (b.kind === 'media') return 'media'
  return 'shortcut'
}

type Props = {
  index: number
  lang: Lang
  showButtonMapping: boolean
  targets: VolumeTarget[]
  binding: ButtonBinding
  hwSliderMute: boolean
  shortcutLedMode: ShortcutLedMode
  shortcutLedLatched?: boolean
  audioSessions: AudioSessionInfo[]
  allAssignments: VolumeTarget[][]
  assignmentError?: string | null
  loadAudioSessions: () => void
  onAssignmentsChange: (targets: VolumeTarget[]) => boolean
  onBindingChange: (b: ButtonBinding) => void
  onHwSliderMuteChange: (v: boolean) => void
  onShortcutLedModeChange: (mode: ShortcutLedMode) => void
  onClose: () => void
}

export function ChannelDrawer({
  index,
  lang,
  showButtonMapping,
  targets,
  binding,
  hwSliderMute,
  shortcutLedMode,
  shortcutLedLatched,
  audioSessions,
  allAssignments,
  assignmentError,
  loadAudioSessions,
  onAssignmentsChange,
  onBindingChange,
  onHwSliderMuteChange,
  onShortcutLedModeChange,
  onClose,
}: Props) {
  const [recording, setRecording] = useState(false)
  const [keysPreview, setKeysPreview] = useState<string[]>([])
  const inputRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    loadAudioSessions()
  }, [loadAudioSessions])

  useEffect(() => {
    if (recording) inputRef.current?.focus()
  }, [recording])

  const hasSystem = targets.some((t) => t.type === 'system')
  const hasMic = targets.some((t) => t.type === 'mic')
  const hasGames = targets.some((t) => t.type === 'category' && t.id === 'gry')

  const normalize = (name?: string) => (name || '').trim().toLowerCase().replace(/\.exe$/i, '')

  const excludedAppNames = useMemo(() => {
    const set = new Set<string>()
    for (const t of targets) {
      if (t.type === 'app') set.add(normalize(t.name))
    }
    return set
  }, [targets])

  const addTarget = (t: VolumeTarget): boolean => {
    const targetAppName = t.type === 'app' ? normalize(t.name) : ''
    const exists = targets.some((x) => {
      if (x.type !== t.type) return false
      if (t.type === 'app' && x.type === 'app') {
        return normalize(x.name) === targetAppName || (t.pid > 0 && x.pid === t.pid)
      }
      if (t.type === 'category' && x.type === 'category') return x.id === t.id
      return true
    })
    if (exists) return false
    return onAssignmentsChange([...targets, t])
  }

  const removeAt = (idx: number) => {
    onAssignmentsChange(targets.filter((_, i) => i !== idx))
  }

  const toggleSpecial = (type: 'system' | 'mic' | 'games') => {
    if (type === 'system') {
      if (hasSystem) onAssignmentsChange(targets.filter((t) => t.type !== 'system'))
      else addTarget({ type: 'system' })
      return
    }
    if (type === 'mic') {
      if (hasMic) onAssignmentsChange(targets.filter((t) => t.type !== 'mic'))
      else addTarget({ type: 'mic' })
      return
    }
    if (hasGames) onAssignmentsChange(targets.filter((t) => !(t.type === 'category' && t.id === 'gry')))
    else addTarget({ type: 'category', id: 'gry' })
  }

  const setMode = (mode: 'none' | 'media' | 'shortcut') => {
    if (mode === 'none') onBindingChange({ kind: 'none' })
    else if (mode === 'media')
      onBindingChange({
        kind: 'media',
        action: binding.kind === 'media' ? binding.action : 'play_pause',
      })
    else
      onBindingChange(
        binding.kind === 'shortcut' ? binding : { kind: 'shortcut', vk: 0, mods: 0, label: '' },
      )
  }

  const setMediaKey = (action: MediaKeyAction) => {
    onBindingChange({ kind: 'media', action })
  }

  const conflicts = (t: VolumeTarget) => {
    const name = targetDisplayName(t, lang).trim().toLowerCase().replace(/\.exe$/i, '')
    for (let j = 0; j < allAssignments.length; j++) {
      if (j === index) continue
      for (const u of allAssignments[j] ?? []) {
        const other = targetDisplayName(u, lang).trim().toLowerCase().replace(/\.exe$/i, '')
        if (name && other === name) return true
        if (t.type === 'app' && u.type === 'app') {
          if (normalize(t.name) && normalize(t.name) === normalize(u.name)) return true
          if (t.pid === u.pid) return true
        }
        if (t.type === 'system' && u.type === 'system') return true
        if (t.type === 'mic' && u.type === 'mic') return true
        if (t.type === 'category' && u.type === 'category' && t.id === u.id) return true
      }
    }
    return false
  }

  return (
    <div
      style={{
        gridColumn: '1 / -1',
        background: 'var(--c-surface)',
        border: '0.5px solid var(--c-border)',
        borderRadius: 10,
        padding: 16,
        marginTop: -4,
      }}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 14 }}>
        <div style={{ fontSize: 12, color: 'var(--c-text-muted)', display: 'flex', alignItems: 'center', gap: 6 }}>
          <Sliders size={13} />
          {lang === 'pl' ? `Konfiguracja suwaka ${index + 1}` : `Configuring slider ${index + 1}`}
        </div>
        <button
          type="button"
          onClick={onClose}
          style={{ background: 'transparent', border: 'none', color: 'var(--c-text-muted)', cursor: 'pointer', display: 'flex' }}
          aria-label="Close"
        >
          <X size={16} />
        </button>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 28 }}>
        <div>
          <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, marginBottom: 8 }}>ROUTING</div>

          <AssignmentRow icon={Monitor} label={lang === 'pl' ? 'System' : 'System'}>
            <Toggle checked={hasSystem} onChange={() => toggleSpecial('system')} />
          </AssignmentRow>
          <AssignmentRow icon={Mic} label={lang === 'pl' ? 'Mikrofon' : 'Mic'}>
            <Toggle checked={hasMic} onChange={() => toggleSpecial('mic')} />
          </AssignmentRow>

          <AssignmentRow icon={Search} label={lang === 'pl' ? 'Aplikacja' : 'App'}>
            <AppSessionCombobox
              lang={lang}
              sessions={audioSessions}
              excludedNames={excludedAppNames}
              onRefresh={loadAudioSessions}
              onPick={(session) =>
                addTarget({ type: 'app', pid: session.pid, name: session.name })
              }
            />
          </AssignmentRow>

          <AssignmentRow icon={Gamepad2} label={lang === 'pl' ? 'Gry' : 'Games'}>
            <button
              type="button"
              onClick={() => toggleSpecial('games')}
              style={{
                fontSize: 11,
                padding: '3px 8px',
                borderRadius: 20,
                cursor: 'pointer',
                background: hasGames ? 'color-mix(in srgb, var(--c-accent) 12%, transparent)' : 'var(--c-elevated)',
                color: hasGames ? ACCENT : 'var(--c-text-muted)',
                border: `0.5px solid ${hasGames ? ACCENT : 'var(--c-border-strong)'}`,
              }}
            >
              {lang === 'pl' ? 'Grupa gier' : 'Games group'}
            </button>
          </AssignmentRow>

          <div style={{ marginTop: 10, paddingTop: 10, borderTop: '0.5px solid var(--c-border)' }}>
            <div style={{ fontSize: 10, color: 'var(--c-text-dim)', marginBottom: 6 }}>ASSIGNED</div>
            <div>
              {targets.length === 0 && (
                <span style={{ fontSize: 11, color: 'var(--c-text-faint)' }}>
                  {lang === 'pl' ? 'Nic jeszcze nie przypisano' : 'Nothing assigned yet'}
                </span>
              )}
              {targets.map((t, i) => (
                <Chip key={i} warn={conflicts(t)} onRemove={() => removeAt(i)}>
                  {targetDisplayName(t, lang)}
                </Chip>
              ))}
            </div>
            {assignmentError && (
              <div style={{ fontSize: 11, color: 'var(--c-danger)', marginTop: 8 }} role="alert">
                {assignmentError}
              </div>
            )}
          </div>
        </div>

        <div>
          <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, marginBottom: 8 }}>BUTTON ACTION</div>
          {showButtonMapping ? (
            <>
              <div style={{ display: 'flex', gap: 4, background: 'var(--c-seg)', borderRadius: 8, padding: 3, marginBottom: 12 }}>
                {(['none', 'media', 'shortcut'] as const).map((m) => (
                  <button
                    key={m}
                    type="button"
                    onClick={() => setMode(m)}
                    style={{
                      flex: 1,
                      fontSize: 11,
                      padding: '6px 0',
                      borderRadius: 6,
                      border: 'none',
                      cursor: 'pointer',
                      background: bindingMode(binding) === m ? 'var(--c-seg-active)' : 'transparent',
                      color: bindingMode(binding) === m ? 'var(--c-text)' : 'var(--c-text-muted)',
                      textTransform: 'capitalize',
                    }}
                  >
                    {m === 'none' ? 'None' : m}
                  </button>
                ))}
              </div>

              {bindingMode(binding) === 'media' && (
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
                  {MEDIA_KEYS.map((k) => {
                    const Icon = k.icon
                    const active = binding.kind === 'media' && binding.action === k.id
                    return (
                      <button
                        key={k.id}
                        type="button"
                        onClick={() => setMediaKey(k.id)}
                        style={{
                          display: 'flex',
                          alignItems: 'center',
                          gap: 6,
                          fontSize: 11,
                          padding: '7px 8px',
                          borderRadius: 6,
                          cursor: 'pointer',
                          background: active ? 'color-mix(in srgb, var(--c-accent) 12%, transparent)' : 'var(--c-seg)',
                          color: active ? ACCENT : 'var(--c-text-secondary)',
                          border: `0.5px solid ${active ? ACCENT : 'var(--c-seg-active)'}`,
                        }}
                      >
                        <Icon size={13} /> {k.label}
                      </button>
                    )
                  })}
                </div>
              )}

              {bindingMode(binding) === 'shortcut' && (
                <div>
                  <button
                    ref={inputRef}
                    type="button"
                    onClick={() => setRecording(true)}
                    onKeyDown={(e) => {
                      if (!recording) return
                      e.preventDefault()
                      e.stopPropagation()
                      if (e.repeat) return
                      if (e.code === 'Escape') {
                        setRecording(false)
                        setKeysPreview([])
                        return
                      }
                      const mods: string[] = []
                      if (e.ctrlKey) mods.push('Ctrl')
                      if (e.shiftKey) mods.push('Shift')
                      if (e.altKey) mods.push('Alt')
                      if (e.metaKey) mods.push('Win')
                      if (isModifierCode(e.code)) {
                        setKeysPreview(mods)
                        return
                      }
                      const vk = keyboardCodeToVk(e.code)
                      if (vk == null) return
                      const mask = modifierMask(e)
                      if (mask === 0 && !shortcutAllowedWithoutModifier(e.code)) return
                      const label = shortcutLabelFromEvent(e)
                      setKeysPreview(label.split('+'))
                      onBindingChange({ kind: 'shortcut', vk, mods: mask, label })
                      setRecording(false)
                    }}
                    onBlur={() => {
                      setRecording(false)
                      setKeysPreview([])
                    }}
                    style={{
                      width: '100%',
                      padding: 10,
                      borderRadius: 8,
                      textAlign: 'center',
                      cursor: 'pointer',
                      background: recording ? 'color-mix(in srgb, var(--c-accent) 8%, transparent)' : 'var(--c-seg)',
                      border: `0.5px solid ${recording ? ACCENT : 'var(--c-seg-active)'}`,
                      color: recording ? ACCENT : 'var(--c-text-secondary)',
                      fontSize: 13,
                      outline: 'none',
                    }}
                  >
                    {recording
                      ? keysPreview.length
                        ? `${keysPreview.join(' + ')} …`
                        : lang === 'pl'
                          ? 'Naciśnij kombinację…'
                          : 'Press a key combination…'
                      : binding.kind === 'shortcut' && binding.label
                        ? binding.label
                        : lang === 'pl'
                          ? 'Kliknij, aby nagrać skrót'
                          : 'Click to record shortcut'}
                  </button>
                  <div style={{ fontSize: 10, color: 'var(--c-text-dim)', marginTop: 6 }}>
                    {lang === 'pl'
                      ? 'Modyfikatory plus jeden klawisz, np. Ctrl+Shift+Alt+J.'
                      : 'Modifiers plus one key, e.g. Ctrl+Shift+Alt+J.'}
                  </div>
                </div>
              )}

              {bindingMode(binding) === 'none' && (
                <div style={{ fontSize: 11, color: 'var(--c-text-dim)', padding: '8px 0' }}>
                  {lang === 'pl'
                    ? 'Przycisk sprzętowy nic nie robi na tym suwaku.'
                    : 'The hardware button does nothing on this slider.'}
                </div>
              )}
            </>
          ) : (
            <div style={{ fontSize: 11, color: 'var(--c-text-dim)', padding: '8px 0' }}>
              {lang === 'pl' ? 'To urządzenie nie ma przycisku na tym suwaku.' : 'No hardware button on this slider.'}
            </div>
          )}

          <div style={{ marginTop: 16, paddingTop: 12, borderTop: '0.5px solid var(--c-border)' }}>
            <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, marginBottom: 8 }}>HARDWARE</div>
            <AssignmentRow icon={VolumeX} label={lang === 'pl' ? 'Mute kanału (app)' : 'Channel mute (app)'}>
              <Toggle checked={hwSliderMute} onChange={onHwSliderMuteChange} />
            </AssignmentRow>
            {!hwSliderMute && (
              <div style={{ marginTop: 8 }}>
                <div style={{ fontSize: 11, color: 'var(--c-text-muted)', marginBottom: 6, display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Radio size={12} />
                  {lang === 'pl' ? 'LED przy skrócie / media' : 'LED on shortcut / media'}
                  {shortcutLedMode === 'toggle' && shortcutLedLatched ? (
                    <span style={{ color: ACCENT, fontSize: 10 }}>● ON</span>
                  ) : null}
                </div>
                <div style={{ display: 'flex', gap: 4, background: 'var(--c-seg)', borderRadius: 8, padding: 3 }}>
                  {(
                    [
                      { id: 'off' as const, pl: 'Off', en: 'Off' },
                      { id: 'momentary' as const, pl: 'Hold', en: 'Hold' },
                      { id: 'toggle' as const, pl: 'Toggle', en: 'Toggle' },
                    ] as const
                  ).map((m) => (
                    <button
                      key={m.id}
                      type="button"
                      onClick={() => onShortcutLedModeChange(m.id)}
                      style={{
                        flex: 1,
                        fontSize: 11,
                        padding: '5px 0',
                        borderRadius: 6,
                        border: 'none',
                        cursor: 'pointer',
                        background: shortcutLedMode === m.id ? 'var(--c-seg-active)' : 'transparent',
                        color: shortcutLedMode === m.id ? 'var(--c-text)' : 'var(--c-text-muted)',
                      }}
                    >
                      {lang === 'pl' ? m.pl : m.en}
                    </button>
                  ))}
                </div>
                <div style={{ fontSize: 10, color: 'var(--c-text-dim)', marginTop: 6 }}>
                  {shortcutLedMode === 'toggle'
                    ? lang === 'pl'
                      ? 'Toggle: dioda zostaje zapalona do kolejnego wciśnięcia (np. mute Discord).'
                      : 'Toggle: LED stays on until pressed again (e.g. Discord mute).'
                    : shortcutLedMode === 'momentary'
                      ? lang === 'pl'
                        ? 'Hold: świeci tylko podczas trzymania przycisku.'
                        : 'Hold: lights only while the button is held.'
                      : lang === 'pl'
                        ? 'Off: bez sterowania LED przy skrócie.'
                        : 'Off: no LED feedback for this shortcut.'}
                </div>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
