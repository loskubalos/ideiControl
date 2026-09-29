import React, { useEffect, useMemo, useRef, useState } from 'react'
import { IconX } from './icons'

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

const MOD_CTRL = 1
const MOD_SHIFT = 2
const MOD_ALT = 4
const MOD_WIN = 8

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

function bindingMode(b: ButtonBinding): 'none' | 'media' | 'shortcut' {
  if (b.kind === 'none') return 'none'
  if (b.kind === 'media') return 'media'
  return 'shortcut'
}

function defaultShortcutLabel(lang: Lang) {
  return lang === 'pl' ? 'Brak skrótu' : 'No shortcut'
}

function targetLabel(t: VolumeTarget, lang: Lang): string {
  if (t.type === 'system') return lang === 'pl' ? 'Głośność systemu' : 'System volume'
  if (t.type === 'mic') return lang === 'pl' ? 'Głośność mikrofonu' : 'Microphone volume'
  if (t.type === 'category') {
    return t.id === 'gry' ? (lang === 'pl' ? 'Gry' : 'Games') : t.id
  }
  return t.name && t.name.trim() ? t.name : lang === 'pl' ? `Aplikacja ${t.pid}` : `App ${t.pid}`
}

function targetDotClass(t: VolumeTarget) {
  if (t.type === 'system') return 'bg-accent'
  if (t.type === 'mic') return 'bg-emerald-500'
  if (t.type === 'app') return 'bg-blue-500'
  return 'bg-orange-500'
}

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

function actionSummary(binding: ButtonBinding, lang: Lang): string {
  if (binding.kind === 'none') return lang === 'pl' ? 'Brak' : 'None'
  if (binding.kind === 'media') {
    if (lang === 'pl') {
      const map: Record<MediaKeyAction, string> = {
        none: 'Brak',
        play_pause: 'Odtwarzaj/Pauza',
        next_track: 'Następny utwór',
        previous_track: 'Poprzedni utwór',
        stop: 'Stop',
        mute: 'Wycisz system',
        volume_up: 'Głośniej',
        volume_down: 'Ciszej',
      }
      return map[binding.action] ?? binding.action
    }
    const opt = MEDIA_ACTION_OPTIONS.find((o) => o.value === binding.action)
    return opt?.label ?? binding.action
  }
  return binding.label.trim() ? binding.label : defaultShortcutLabel(lang)
}

function IconChevronDown({ className = '' }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="m6 9 6 6 6-6" />
    </svg>
  )
}

export type V0ControllerCardProps = {
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
  onAssignmentsChange: (targets: VolumeTarget[]) => boolean
  assignmentError?: string | null
}

function Toggle({
  checked,
  onChange,
  label,
}: {
  checked: boolean
  onChange: (v: boolean) => void
  label?: string
}) {
  return (
    <label className="flex items-center justify-between gap-3 cursor-pointer select-none">
      {label ? (
        <span className="text-sm text-muted-foreground">{label}</span>
      ) : (
        <span className="text-sm text-muted-foreground" />
      )}
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
        className={`relative h-5 w-9 rounded-full transition-colors duration-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring ${
          checked ? 'bg-accent' : 'bg-muted'
        }`}
      >
        <span
          className={`absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-foreground transition-transform duration-200 ${
            checked ? 'translate-x-4' : 'translate-x-0'
          }`}
        />
      </button>
    </label>
  )
}

export function V0ControllerCard({
  index,
  sliderIndex: _sliderIndex,
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
  assignmentError,
}: V0ControllerCardProps) {
  const [expanded, setExpanded] = useState(false)
  const [addOpen, setAddOpen] = useState(false)
  const [recordingShortcut, setRecordingShortcut] = useState(false)
  const captureRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (recordingShortcut) captureRef.current?.focus()
  }, [recordingShortcut])

  const [addSearch, setAddSearch] = useState('')
  const percent = Math.round((value / 1023) * 100)

  const normalizeAppName = (name?: string) => (name || '').trim().toLowerCase().replace(/\.exe$/, '')
  const activeSessions = useMemo(() => audioSessions.filter((s) => s.is_active), [audioSessions])

  const allFiltered = useMemo(() => {
    const q = addSearch.trim().toLowerCase()
    if (!q) return audioSessions
    return audioSessions.filter((s) => (s.name || '').toLowerCase().includes(q) || String(s.pid).includes(addSearch.trim()))
  }, [addSearch, audioSessions])

  const removeTarget = (idx: number) => {
    const next = targets.filter((_, i) => i !== idx)
    onAssignmentsChange(next)
  }

  const addTarget = (t: VolumeTarget) => {
    const targetAppName = t.type === 'app' ? normalizeAppName(t.name) : ''
    const exists = targets.some(
      (x) =>
        x.type === t.type &&
        (x.type !== 'app'
          ? x.type !== 'category' || (x as { id: string }).id === (t as { id: string }).id
          : normalizeAppName((x as { name?: string }).name) === targetAppName || (x as { pid: number }).pid === (t as { pid: number }).pid),
    )
    if (exists) return
    const ok = onAssignmentsChange([...targets, t])
    if (ok) setAddOpen(false)
  }

  const bindingLabel = actionSummary(binding, lang)

  return (
    // allow popovers/dropdowns to render outside the card bounds
    <article className="rounded-xl border border-border bg-card flex flex-col overflow-visible shadow-sm hover:shadow-md transition-shadow">
      <div className="px-4 pt-4 pb-3">
        <div className="flex items-center justify-between mb-3">
          <span className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">
            {lang === 'pl' ? 'Suwak' : 'Slider'} {index}
          </span>
          <span className="text-2xl font-bold tabular-nums text-foreground">{percent}%</span>
        </div>

        <div className="h-1.5 rounded-full bg-muted overflow-hidden">
          <div className="h-full rounded-full bg-accent transition-all duration-300" style={{ width: `${percent}%` }} />
        </div>
      </div>

      <div className="px-4 pb-3">
        {targets.length === 0 ? (
          <span className="text-xs text-muted-foreground italic">{lang === 'pl' ? 'Bez przypisań' : 'Unassigned'}</span>
        ) : (
          <div className="flex flex-wrap gap-1.5">
            {targets.map((t, i) => (
              <span key={i} className="inline-flex items-center gap-1 rounded-md bg-muted px-2 py-0.5 text-xs text-foreground">
                <span className={`h-1.5 w-1.5 rounded-full shrink-0 ${targetDotClass(t)}`} />
                {targetLabel(t, lang)}
              </span>
            ))}
          </div>
        )}
      </div>

      <div className="px-4 pb-3">
        <span className="inline-flex items-center gap-1.5 rounded-md border border-border px-2 py-0.5 text-xs text-muted-foreground">
          <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/40" />
          {bindingLabel}
        </span>
      </div>

      <div className="mt-auto">
        <button
          type="button"
          onClick={() => setExpanded((o) => !o)}
          className="w-full flex items-center justify-between border-t border-border px-4 py-2.5 text-xs text-muted-foreground hover:bg-muted/40 hover:text-foreground transition-colors"
        >
          <span>{expanded ? (lang === 'pl' ? 'Zamknij' : 'Hide controls') : lang === 'pl' ? 'Konfiguruj' : 'Configure'}</span>
          <IconChevronDown className={`h-3.5 w-3.5 transition-transform duration-200 ${expanded ? 'rotate-180' : ''}`} />
        </button>
      </div>

      {expanded && (
        <div className="border-t border-border bg-background/30 px-4 py-4 space-y-4">
          {showButtonMapping && (
            <section className="space-y-3">
              <p className="text-[11px] font-semibold uppercase tracking-widest text-muted-foreground mb-2">
                {lang === 'pl' ? `Przycisk ${index}` : `Button ${index} action`}
              </p>

              <div className="flex flex-col gap-2">
                <label className="text-xs text-muted-foreground">{lang === 'pl' ? 'Akcja w Windows' : 'Action in Windows'}</label>
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
                        binding.kind === 'shortcut' ? binding : { kind: 'shortcut', vk: 0, mods: 0, label: '' },
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
                    <label className="text-xs text-muted-foreground">{lang === 'pl' ? 'Wybierz klawisz' : 'Choose key'}</label>
                    <select
                      value={binding.kind === 'media' ? binding.action : 'play_pause'}
                      onChange={(e) => onBindingChange({ kind: 'media', action: e.target.value as MediaKeyAction })}
                      className="w-full rounded-lg border border-border bg-card px-2 py-1.5 text-sm text-foreground outline-none focus:border-accent"
                    >
                      {MEDIA_ACTION_OPTIONS.filter((o) => o.value !== 'none').map((o) => (
                        <option key={o.value} value={o.value}>
                          {lang === 'pl' ? o.label : o.label}
                        </option>
                      ))}
                    </select>
                  </>
                )}

                {bindingMode(binding) === 'shortcut' && (
                  <div className="space-y-2">
                    <p className="text-xs text-muted-foreground">
                      {lang === 'pl' ? 'Nagraj i naciśnij kombinację. Esc anuluje.' : 'Record and press the combination. Esc cancels.'}
                    </p>
                    <div className="rounded-md border border-border bg-card px-2 py-1.5 font-mono text-sm text-foreground">
                      {binding.kind === 'shortcut' && binding.label.trim() ? binding.label : '— brak —'}
                    </div>

                    <button
                      type="button"
                      onClick={() => setRecordingShortcut((r) => !r)}
                      className={`w-full rounded-lg border px-2 py-1.5 text-sm font-medium transition-colors ${
                        recordingShortcut ? 'border-accent bg-accent/15 text-accent' : 'border-border bg-background text-foreground hover:bg-muted'
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
                        {lang === 'pl' ? 'Naciśnij teraz' : 'Press now'}
                      </div>
                    )}
                  </div>
                )}
              </div>
            </section>
          )}

          <section className="space-y-2">
            <Toggle
              checked={hwSliderMute}
              onChange={onHwSliderMuteChange}
              label={lang === 'pl' ? 'Wycisz suwak (mute)' : 'Mute on device'}
            />
            {!hwSliderMute && (
              <Toggle
                checked={shortcutNeoOnShortcut}
                onChange={onShortcutNeoChange}
                label={
                  lang === 'pl'
                    ? 'LED przy wyciszeniu ze skrótu (np. Discord)'
                    : 'LED on shortcut mute (e.g. Discord)'
                }
              />
            )}
          </section>

          <section className="space-y-2">
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              {lang === 'pl' ? 'Przypisania' : 'Assignments'}
            </p>

            <div className="space-y-2">
              {targets.map((t, i) => (
                <div key={i} className="flex items-center justify-between rounded-lg border border-border bg-background p-3">
                  <div className="flex items-center gap-2 min-w-0">
                    <div className={`h-2 w-2 rounded-full shrink-0 ${targetDotClass(t)}`} />
                    <span className="text-sm text-foreground truncate">{targetLabel(t, lang)}</span>
                  </div>
                  <button type="button" onClick={() => removeTarget(i)} className="shrink-0 text-muted-foreground hover:text-destructive focus:outline-none focus:ring-1 focus:ring-accent rounded p-0.5" aria-label="Remove">
                    <IconX className="h-4 w-4" />
                  </button>
                </div>
              ))}

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

                {assignmentError && (
                  <p className="mt-2 text-xs text-destructive break-words" role="alert">
                    {assignmentError}
                  </p>
                )}

                {addOpen && (
                  <>
                    <div className="absolute z-[100] top-full left-0 right-0 mt-1.5 rounded-lg border border-border bg-card shadow-xl overflow-hidden flex flex-col w-full min-w-[240px] max-h-[min(480px,80vh)]">
                      <div className="p-2 flex flex-col gap-1 shrink-0">
                        <button type="button" className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 transition-colors" onClick={() => addTarget({ type: 'system' })}>
                          {lang === 'pl' ? 'Głośność systemu' : 'System volume'}
                        </button>
                        <button type="button" className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 transition-colors" onClick={() => addTarget({ type: 'mic' })}>
                          {lang === 'pl' ? 'Głośność mikrofonu' : 'Microphone volume'}
                        </button>
                        <div className="border-t border-border my-1" />
                        <button type="button" className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 flex items-center gap-2 transition-colors" onClick={() => addTarget({ type: 'category', id: 'gry' })}>
                          <span className="shrink-0">🎮</span> {lang === 'pl' ? 'Gry' : 'Games'}
                        </button>
                        <div className="border-t border-border my-1" />
                        <p className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground px-3 pt-1 pb-0.5">
                          {lang === 'pl' ? 'Aktualnie grane' : 'Currently playing'}
                        </p>
                        {activeSessions.length === 0 ? (
                          <p className="text-xs text-muted-foreground px-3 py-2">{lang === 'pl' ? 'Brak aktywnych' : 'None active'}</p>
                        ) : (
                          activeSessions.slice(0, 8).map((s) => (
                            <button
                              type="button"
                              key={s.pid}
                              className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 truncate transition-colors"
                              onClick={() => addTarget({ type: 'app', pid: s.pid, name: s.name })}
                            >
                              {s.name || `PID ${s.pid}`}
                            </button>
                          ))
                        )}
                      </div>

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
                              {addSearch.trim()
                                ? lang === 'pl'
                                  ? 'Brak wyników'
                                  : 'No results'
                                : lang === 'pl'
                                  ? 'Ładowanie…'
                                  : 'Loading…'}
                            </p>
                          ) : (
                            <div className="py-0.5 space-y-0.5">
                              {allFiltered.map((s) => (
                                <button
                                  type="button"
                                  key={s.pid}
                                  className="w-full text-left text-sm text-foreground hover:bg-muted rounded-md px-3 py-2 truncate flex items-center gap-2 transition-colors"
                                  onClick={() => addTarget({ type: 'app', pid: s.pid, name: s.name })}
                                >
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
          </section>
        </div>
      )}
    </article>
  )
}

