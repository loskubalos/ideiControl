import { useEffect, useRef, useState } from 'react'
import { Check, Copy, Pencil, Plus, Trash2 } from 'lucide-react'
import { ACCENT, iconBtnStyle } from './shared'
import type { Lang, Profile } from './types'

type Props = {
  modelLabel: string
  lang: Lang
  profiles: Profile[]
  activeId: string | null
  disabled?: boolean
  savedHint?: boolean
  onSelect: (id: string) => void
  onRename: (id: string, name: string) => void
  onDuplicate: (id: string) => void
  onDelete: (id: string) => void
  onAdd: (name: string) => void
}

const LABEL = {
  pl: {
    rename: 'Zmie\u0144 nazw\u0119',
    duplicate: 'Duplikuj profil',
    delete: 'Usu\u0144',
    deleteConfirm: 'Kliknij ponownie, aby usun\u0105\u0107',
    deleteBlocked: 'Nie mo\u017cna usun\u0105\u0107 ostatniego/domy\u015blnego profilu',
    saved: 'Zapisano',
    noProfiles: 'Brak profili',
    profile: 'Profil',
    newProfile: 'Nowy profil',
    namePlaceholder: 'Nazwa profilu\u2026',
    defaultSuffix: ' (domy\u015blny)',
  },
  en: {
    rename: 'Rename',
    duplicate: 'Duplicate profile',
    delete: 'Delete',
    deleteConfirm: 'Click again to confirm delete',
    deleteBlocked: 'Cannot delete the last/default profile',
    saved: 'Saved',
    noProfiles: 'No profiles',
    profile: 'Profile',
    newProfile: 'New profile',
    namePlaceholder: 'Profile name\u2026',
    defaultSuffix: ' (default)',
  },
} as const

export function PresetsRail({
  modelLabel,
  lang,
  profiles,
  activeId,
  disabled,
  savedHint,
  onSelect,
  onRename,
  onDuplicate,
  onDelete,
  onAdd,
}: Props) {
  const [editing, setEditing] = useState(false)
  const [name, setName] = useState('')
  const [adding, setAdding] = useState(false)
  const [newName, setNewName] = useState('')
  const [confirmDelete, setConfirmDelete] = useState(false)
  const confirmTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const t = LABEL[lang]

  const active = profiles.find((p) => p.id === activeId) ?? null
  const canDelete = profiles.length > 1 && !(active?.is_default)

  useEffect(() => {
    setEditing(false)
    setConfirmDelete(false)
    if (confirmTimer.current) clearTimeout(confirmTimer.current)
  }, [activeId])

  useEffect(() => {
    return () => {
      if (confirmTimer.current) clearTimeout(confirmTimer.current)
    }
  }, [])

  const startRename = () => {
    if (!active) return
    setName(active.name)
    setEditing(true)
  }

  const commitRename = () => {
    if (!active) return
    onRename(active.id, name)
    setEditing(false)
  }

  const addProfile = () => {
    const trimmed = newName.trim()
    if (!trimmed) {
      setAdding(false)
      return
    }
    onAdd(trimmed)
    setNewName('')
    setAdding(false)
  }

  const requestDelete = () => {
    if (!active || !canDelete) return
    if (confirmDelete) {
      onDelete(active.id)
      setConfirmDelete(false)
      return
    }
    setConfirmDelete(true)
    if (confirmTimer.current) clearTimeout(confirmTimer.current)
    confirmTimer.current = setTimeout(() => setConfirmDelete(false), 2000)
  }

  return (
    <div
      style={{
        width: 220,
        background: 'var(--c-surface)',
        border: '0.5px solid var(--c-border)',
        borderRadius: 10,
        padding: 12,
        flexShrink: 0,
        opacity: disabled ? 0.55 : 1,
        pointerEvents: disabled ? 'none' : 'auto',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 8,
          marginBottom: 8,
        }}
      >
        <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4 }}>
          PROFILES · {modelLabel}
        </div>
        {savedHint && (
          <span style={{ fontSize: 10, color: ACCENT, letterSpacing: 0.2 }}>
            {t.saved}
          </span>
        )}
      </div>

      {editing && active ? (
        <input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={commitRename}
          onKeyDown={(e) => {
            if (e.key === 'Enter') commitRename()
            if (e.key === 'Escape') setEditing(false)
          }}
          style={{
            width: '100%',
            background: 'var(--c-elevated)',
            border: `0.5px solid ${ACCENT}`,
            borderRadius: 6,
            padding: '6px 8px',
            color: 'var(--c-text)',
            fontSize: 12,
            outline: 'none',
            marginBottom: 8,
          }}
        />
      ) : (
        <select
          value={activeId ?? ''}
          onChange={(e) => {
            const id = e.target.value
            if (id) onSelect(id)
          }}
          disabled={!profiles.length}
          aria-label={t.profile}
          style={{
            width: '100%',
            background: 'var(--c-elevated)',
            border: '0.5px solid var(--c-border)',
            borderRadius: 6,
            padding: '6px 8px',
            color: 'var(--c-text)',
            fontSize: 12.5,
            outline: 'none',
            marginBottom: 8,
            cursor: 'pointer',
          }}
        >
          {profiles.length === 0 && <option value="">{t.noProfiles}</option>}
          {profiles.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
              {p.is_default ? t.defaultSuffix : ''}
            </option>
          ))}
        </select>
      )}

      <div style={{ display: 'flex', gap: 2, marginBottom: 8 }}>
        <button
          type="button"
          onClick={startRename}
          disabled={!active}
          aria-label={t.rename}
          title={t.rename}
          style={iconBtnStyle}
        >
          <Pencil size={13} />
        </button>
        <button
          type="button"
          onClick={() => active && onDuplicate(active.id)}
          disabled={!active}
          aria-label={t.duplicate}
          title={t.duplicate}
          style={iconBtnStyle}
        >
          <Copy size={13} />
        </button>
        <button
          type="button"
          onClick={requestDelete}
          disabled={!canDelete}
          aria-label={t.delete}
          title={!canDelete ? t.deleteBlocked : confirmDelete ? t.deleteConfirm : t.delete}
          style={{
            ...iconBtnStyle,
            color: confirmDelete ? 'var(--c-danger)' : 'var(--c-text-muted)',
            opacity: canDelete ? 1 : 0.35,
          }}
        >
          {confirmDelete ? <Check size={13} /> : <Trash2 size={13} />}
        </button>
      </div>

      <div style={{ borderTop: '0.5px solid var(--c-border)', paddingTop: 8 }}>
        {adding ? (
          <input
            autoFocus
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onBlur={addProfile}
            onKeyDown={(e) => e.key === 'Enter' && addProfile()}
            placeholder={t.namePlaceholder}
            style={{
              width: '100%',
              background: 'var(--c-elevated)',
              border: `0.5px solid ${ACCENT}`,
              borderRadius: 6,
              padding: '5px 8px',
              color: 'var(--c-text)',
              fontSize: 12,
              outline: 'none',
            }}
          />
        ) : (
          <button
            type="button"
            onClick={() => setAdding(true)}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              fontSize: 12,
              color: 'var(--c-text-muted)',
              background: 'transparent',
              border: 'none',
              cursor: 'pointer',
              padding: '4px 2px',
            }}
          >
            <Plus size={13} /> {t.newProfile}
          </button>
        )}
      </div>
    </div>
  )
}
