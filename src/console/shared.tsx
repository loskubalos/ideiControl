import type { CSSProperties, ReactNode } from 'react'
import {
  AlertTriangle,
  Gamepad2,
  Keyboard,
  Mic,
  Monitor,
  Play,
  SkipBack,
  SkipForward,
  Square,
  Volume2,
  VolumeX,
  X,
} from 'lucide-react'
import type { ButtonBinding, MediaKeyAction, VolumeTarget } from './types'

/** Accent fallback when CSS vars are unavailable (e.g. outside app-container). */
export const ACCENT = 'var(--c-accent, #3ecf8e)'

export const APP_COLORS: Record<string, string> = {
  Games: '#7c6a9c',
  Gry: '#7c6a9c',
  javaw: '#5c9c7c',
  'YouTube Music': '#c96a8a',
  'Microsoft.Media.Player': '#6a8ec9',
  vlc: '#c98a4a',
  wmplayer: '#c9a04a',
  opera: '#c95a4a',
  msedge: '#4a9cc9',
  Discord: '#6a7ec9',
  Microphone: '#c9946a',
  System: '#4a9c8a',
  Spotify: '#5c9c7c',
  Chrome: '#c96a4a',
  Steam: '#6a7ec9',
  'OBS Studio': '#8a6ac9',
  Slack: '#c96a8a',
}

export const MEDIA_KEYS: { id: MediaKeyAction; label: string; icon: typeof Play }[] = [
  { id: 'play_pause', label: 'Play / pause', icon: Play },
  { id: 'next_track', label: 'Next track', icon: SkipForward },
  { id: 'previous_track', label: 'Previous track', icon: SkipBack },
  { id: 'stop', label: 'Stop', icon: Square },
  { id: 'mute', label: 'Mute', icon: VolumeX },
  { id: 'volume_up', label: 'Volume up', icon: Volume2 },
  { id: 'volume_down', label: 'Volume down', icon: Volume2 },
]

export function targetDisplayName(t: VolumeTarget, lang: 'pl' | 'en'): string {
  if (t.type === 'system') return lang === 'pl' ? 'System' : 'System'
  if (t.type === 'mic') return lang === 'pl' ? 'Mikrofon' : 'Microphone'
  if (t.type === 'category') return t.id === 'gry' ? (lang === 'pl' ? 'Gry' : 'Games') : t.id
  return t.name && t.name.trim() ? t.name : lang === 'pl' ? `Aplikacja ${t.pid}` : `App ${t.pid}`
}

export function targetKind(t: VolumeTarget): 'system' | 'mic' | 'games' | 'app' {
  if (t.type === 'system') return 'system'
  if (t.type === 'mic') return 'mic'
  if (t.type === 'category') return 'games'
  return 'app'
}

export function ActionSummary({ action, lang }: { action: ButtonBinding; lang: 'pl' | 'en' }) {
  if (action.kind === 'none') {
    return <span style={{ color: 'var(--c-text-dim)' }}>{lang === 'pl' ? 'Brak akcji' : 'No action'}</span>
  }
  if (action.kind === 'media') {
    const k = MEDIA_KEYS.find((m) => m.id === action.action)
    const Icon = k?.icon || Play
    return (
      <>
        <Icon size={12} strokeWidth={2} />
        <span>{k?.label ?? action.action}</span>
      </>
    )
  }
  return (
    <>
      <Keyboard size={12} strokeWidth={2} />
      <span style={{ textAlign: 'center' }}>{action.label || (lang === 'pl' ? 'Nie ustawiono' : 'Not set')}</span>
    </>
  )
}

export function Avatar({ name, kind }: { name: string; kind: 'system' | 'mic' | 'games' | 'app' }) {
  const color = APP_COLORS[name] || '#555'
  const Icon = kind === 'mic' ? Mic : kind === 'system' ? Monitor : kind === 'games' ? Gamepad2 : null
  return (
    <span
      style={{
        width: 14,
        height: 14,
        borderRadius: 4,
        background: color,
        flexShrink: 0,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      {Icon && <Icon size={9} color="#fff" strokeWidth={2.5} />}
    </span>
  )
}

export function AssignmentRow({
  icon: Icon,
  label,
  children,
}: {
  icon: typeof Monitor
  label: string
  children: ReactNode
}) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 0' }}>
      <Icon size={14} color="var(--c-icon)" style={{ flexShrink: 0 }} />
      <span style={{ fontSize: 12, color: 'var(--c-text-secondary)', width: 78, flexShrink: 0 }}>{label}</span>
      <div style={{ flex: 1, minWidth: 0, position: 'relative' }}>{children}</div>
    </div>
  )
}

export function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      style={{
        width: 32,
        height: 18,
        borderRadius: 10,
        border: 'none',
        cursor: 'pointer',
        background: checked ? 'var(--c-accent)' : 'var(--c-toggle-off)',
        position: 'relative',
        transition: 'background 0.15s',
        flexShrink: 0,
      }}
    >
      <span
        style={{
          position: 'absolute',
          top: 2,
          left: checked ? 16 : 2,
          width: 14,
          height: 14,
          borderRadius: '50%',
          background: 'var(--c-toggle-knob)',
          transition: 'left 0.15s',
          boxShadow: '0 1px 2px rgba(0,0,0,0.15)',
        }}
      />
    </button>
  )
}

export function Chip({
  children,
  onRemove,
  warn,
}: {
  children: ReactNode
  onRemove: () => void
  warn?: boolean
}) {
  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 5,
        background: warn ? 'var(--c-warn-bg)' : 'var(--c-chip)',
        border: `0.5px solid ${warn ? 'var(--c-warn-border)' : 'var(--c-border-strong)'}`,
        borderRadius: 20,
        padding: '3px 6px 3px 10px',
        fontSize: 11,
        color: warn ? 'var(--c-warn)' : 'var(--c-text-secondary)',
        margin: '2px 4px 2px 0',
      }}
    >
      {warn && <AlertTriangle size={10} />}
      {children}
      <button
        type="button"
        onClick={onRemove}
        aria-label="Remove"
        style={{ background: 'transparent', border: 'none', color: 'inherit', cursor: 'pointer', padding: 2, display: 'flex' }}
      >
        <X size={11} />
      </button>
    </span>
  )
}

export const iconBtnStyle: CSSProperties = {
  background: 'transparent',
  border: 'none',
  color: 'var(--c-text-muted)',
  cursor: 'pointer',
  padding: 4,
  display: 'flex',
}
