import { Settings, AlertTriangle } from 'lucide-react'
import { ActionSummary, Avatar, targetDisplayName, targetKind } from './shared'
import type { ButtonBinding, Lang, VolumeTarget } from './types'

type Props = {
  index: number
  value: number
  targets: VolumeTarget[]
  binding: ButtonBinding
  isOpen: boolean
  lang: Lang
  allAssignments: VolumeTarget[][]
  onToggle: () => void
}

function targetConflicts(name: string, sliderIndex: number, all: VolumeTarget[][], lang: Lang): boolean {
  const norm = name.trim().toLowerCase().replace(/\.exe$/i, '')
  if (!norm) return false
  for (let j = 0; j < all.length; j++) {
    if (j === sliderIndex) continue
    for (const t of all[j] ?? []) {
      const other = targetDisplayName(t, lang).trim().toLowerCase().replace(/\.exe$/i, '')
      if (other && other === norm) return true
      if (t.type === 'app' && (t.name || '').trim().toLowerCase().replace(/\.exe$/i, '') === norm) return true
    }
  }
  return false
}

export function ChannelStrip({
  index,
  value,
  targets,
  binding,
  isOpen,
  lang,
  allAssignments,
  onToggle,
}: Props) {
  const percent = Math.round((value / 1023) * 100)
  const visible = targets.slice(0, 3)
  const extra = targets.length - 3

  return (
    <div
      style={{
        background: 'var(--c-card)',
        border: `0.5px solid ${isOpen ? 'var(--c-accent)' : 'var(--c-border)'}`,
        borderRadius: 10,
        padding: '12px 10px',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 10,
        position: 'relative',
        transition: 'border-color 0.15s',
      }}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-label={`Configure slider ${index + 1}`}
        style={{
          position: 'absolute',
          top: 8,
          right: 8,
          background: 'transparent',
          border: 'none',
          padding: 4,
          borderRadius: 6,
          cursor: 'pointer',
          color: isOpen ? 'var(--c-accent)' : 'var(--c-text-muted)',
          opacity: isOpen ? 1 : 0.65,
          transition: 'opacity 0.15s, color 0.15s',
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.opacity = '1'
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.opacity = isOpen ? '1' : '0.65'
        }}
      >
        <Settings size={15} />
      </button>

      <div style={{ fontSize: 10, color: 'var(--c-text-dim)', alignSelf: 'flex-start', letterSpacing: 0.4 }}>
        {lang === 'pl' ? 'SUWAK' : 'SLIDER'} {index + 1}
      </div>

      <div style={{ width: '100%', display: 'flex', flexDirection: 'column', gap: 4, minHeight: 44 }}>
        {targets.length === 0 && (
          <div style={{ fontSize: 11, color: 'var(--c-text-faint)' }}>
            {lang === 'pl' ? 'Nie przypisano' : 'Not assigned'}
          </div>
        )}
        {visible.map((t, i) => {
          const name = targetDisplayName(t, lang)
          const warn = targetConflicts(name, index, allAssignments, lang)
          return (
            <div
              key={i}
              style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, color: 'var(--c-text-secondary)' }}
            >
              <Avatar name={name} kind={targetKind(t)} />
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{name}</span>
              {warn && <AlertTriangle size={11} color="var(--c-warn)" style={{ flexShrink: 0 }} />}
            </div>
          )
        })}
        {extra > 0 && <div style={{ fontSize: 10, color: 'var(--c-text-dim)' }}>+{extra} more</div>}
      </div>

      <div
        style={{
          width: 34,
          height: 150,
          background: 'var(--c-elevated)',
          borderRadius: 17,
          position: 'relative',
          overflow: 'hidden',
          marginTop: 2,
        }}
      >
        <div
          style={{
            position: 'absolute',
            bottom: 0,
            width: '100%',
            height: `${percent}%`,
            background: 'var(--c-accent)',
            transition: 'height 0.2s',
          }}
        />
      </div>
      <div style={{ fontSize: 13, fontVariantNumeric: 'tabular-nums', color: 'var(--c-text)' }}>{percent}%</div>

      <div
        style={{
          width: '100%',
          background: 'var(--c-elevated)',
          borderRadius: 8,
          padding: '5px 6px',
          display: 'flex',
          alignItems: 'center',
          gap: 5,
          justifyContent: 'center',
          color: 'var(--c-text-muted)',
          fontSize: 10,
          minHeight: 22,
        }}
      >
        <ActionSummary action={binding} lang={lang} />
      </div>
    </div>
  )
}
