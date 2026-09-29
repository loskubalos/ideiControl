import { useEffect, useMemo, useRef, useState } from 'react'
import { ChevronDown, Plus } from 'lucide-react'
import { ACCENT } from './shared'
import type { AudioSessionInfo, Lang } from './types'

type Props = {
  lang: Lang
  sessions: AudioSessionInfo[]
  excludedNames: Set<string>
  onPick: (session: { pid: number; name: string }) => boolean
  onRefresh: () => void
}

function normalize(name?: string) {
  return (name || '').trim().toLowerCase().replace(/\.exe$/i, '')
}

function labelOf(s: AudioSessionInfo): { primary: string; secondary?: string } {
  const exe = s.exe_name || (s.name.toLowerCase().endsWith('.exe') ? s.name : undefined)
  const friendly = s.display_name?.trim()
  if (friendly && exe && normalize(friendly) !== normalize(exe)) {
    return { primary: friendly, secondary: exe }
  }
  if (friendly) return { primary: friendly, secondary: exe || undefined }
  if (exe) return { primary: exe.replace(/\.exe$/i, ''), secondary: exe }
  return { primary: s.name || `PID ${s.pid}` }
}

export function AppSessionCombobox({ lang, sessions, excludedNames, onPick, onRefresh }: Props) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const rootRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onDoc)
    return () => document.removeEventListener('mousedown', onDoc)
  }, [open])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    return sessions.filter((s) => {
      if (excludedNames.has(normalize(s.name)) || excludedNames.has(normalize(s.exe_name))) {
        return false
      }
      if (!q) return true
      const hay = `${s.name} ${s.exe_name ?? ''} ${s.display_name ?? ''} ${s.pid}`.toLowerCase()
      return hay.includes(q)
    })
  }, [sessions, query, excludedNames])

  const playing = filtered.filter((s) => !!s.is_playing)
  const rest = filtered.filter((s) => !s.is_playing)

  const tryCustom = () => {
    const raw = query.trim()
    if (!raw) return
    const name = raw.toLowerCase().endsWith('.exe') ? raw : raw
    if (excludedNames.has(normalize(name))) return
    const ok = onPick({ pid: 0, name })
    if (ok) {
      setQuery('')
      setOpen(false)
    }
  }

  const pick = (s: AudioSessionInfo) => {
    const name = s.exe_name || s.name
    const ok = onPick({ pid: s.pid, name })
    if (ok) {
      setQuery('')
      setOpen(false)
    }
  }

  const renderRow = (s: AudioSessionInfo) => {
    const { primary, secondary } = labelOf(s)
    const playingDot = !!s.is_playing
    return (
      <button
        key={`${s.pid}-${s.name}`}
        type="button"
        onClick={() => pick(s)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          width: '100%',
          textAlign: 'left',
          background: 'transparent',
          border: 'none',
          color: 'var(--c-text-secondary)',
          fontSize: 12,
          padding: '7px 10px',
          cursor: 'pointer',
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = 'var(--c-elevated)'
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = 'transparent'
        }}
      >
        <span
          style={{
            width: 7,
            height: 7,
            borderRadius: '50%',
            flexShrink: 0,
            background: playingDot ? ACCENT : 'var(--c-text-faint)',
            boxShadow: playingDot ? `0 0 6px ${ACCENT}` : 'none',
          }}
        />
        <span style={{ flex: 1, minWidth: 0 }}>
          <span style={{ color: 'var(--c-text)', display: 'block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {primary}
            {s.is_game ? (
              <span style={{ color: 'var(--c-text-dim)', fontSize: 10 }}> · game</span>
            ) : null}
          </span>
          {secondary ? (
            <span style={{ fontSize: 10, color: 'var(--c-text-dim)', display: 'block', overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {secondary}
            </span>
          ) : null}
        </span>
      </button>
    )
  }

  const section = (title: string, items: AudioSessionInfo[]) => {
    if (items.length === 0) return null
    return (
      <div>
        <div
          style={{
            fontSize: 10,
            color: 'var(--c-text-dim)',
            letterSpacing: 0.35,
            padding: '6px 10px 4px',
            textTransform: 'uppercase',
          }}
        >
          {title}
        </div>
        {items.map(renderRow)}
      </div>
    )
  }

  const customHint =
    query.trim().length > 0 &&
    !filtered.some((s) => normalize(s.name) === normalize(query) || normalize(s.exe_name) === normalize(query))

  return (
    <div ref={rootRef} style={{ position: 'relative', width: '100%' }}>
      <div style={{ display: 'flex', gap: 4 }}>
        <div style={{ flex: 1, position: 'relative' }}>
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              if (!open) {
                setOpen(true)
                onRefresh()
              }
            }}
            onFocus={() => {
              setOpen(true)
              onRefresh()
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                if (filtered[0]) pick(filtered[0])
                else tryCustom()
              } else if (e.key === 'Escape') {
                setOpen(false)
              }
            }}
            placeholder={lang === 'pl' ? 'Szukaj lub wpisz proces…' : 'Search or type process…'}
            style={{
              width: '100%',
              background: 'var(--c-elevated)',
              border: '0.5px solid var(--c-border-strong)',
              borderRadius: 6,
              padding: '5px 28px 5px 8px',
              fontSize: 12,
              color: 'var(--c-text)',
              outline: 'none',
            }}
            aria-expanded={open}
            aria-autocomplete="list"
            role="combobox"
          />
          <ChevronDown
            size={13}
            style={{
              position: 'absolute',
              right: 8,
              top: '50%',
              transform: `translateY(-50%) ${open ? 'rotate(180deg)' : ''}`,
              color: 'var(--c-text-dim)',
              pointerEvents: 'none',
              transition: 'transform 0.15s',
            }}
          />
        </div>
      </div>

      {open && (
        <div
          style={{
            position: 'absolute',
            zIndex: 40,
            left: 0,
            right: 0,
            top: '100%',
            marginTop: 4,
            background: 'var(--c-popover, var(--c-seg))',
            border: '0.5px solid var(--c-border-strong)',
            borderRadius: 8,
            boxShadow: 'var(--c-shadow)',
            maxHeight: 240,
            overflowY: 'auto',
          }}
          role="listbox"
        >
          {section(
            lang === 'pl' ? 'Teraz odtwarza dźwięk' : 'Now playing audio',
            playing,
          )}
          {section(
            lang === 'pl' ? 'Otwarte aplikacje audio' : 'Open audio apps',
            rest,
          )}
          {filtered.length === 0 && !customHint && (
            <div style={{ fontSize: 11, color: 'var(--c-text-dim)', padding: 10 }}>
              {lang === 'pl' ? 'Brak sesji audio — wpisz nazwę ręcznie.' : 'No audio sessions — type a name manually.'}
            </div>
          )}
          {customHint && (
            <button
              type="button"
              onClick={tryCustom}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 6,
                width: '100%',
                textAlign: 'left',
                background: 'transparent',
                border: 'none',
                borderTop: filtered.length ? '0.5px solid var(--c-border)' : 'none',
                color: ACCENT,
                fontSize: 12,
                padding: '8px 10px',
                cursor: 'pointer',
              }}
            >
              <Plus size={13} />
              {lang === 'pl' ? `Dodaj „${query.trim()}”` : `Add “${query.trim()}”`}
            </button>
          )}
        </div>
      )}
    </div>
  )
}
