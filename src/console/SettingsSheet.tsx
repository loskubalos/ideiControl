import { useState } from 'react'
import { ChevronRight, Download, Monitor, Radio, X } from 'lucide-react'
import { AssignmentRow, Toggle } from './shared'
import type { AppUpdateInfo, Lang, UpdateDownloadProgress } from './types'

type Props = {
  lang: Lang
  theme: 'system' | 'light' | 'dark'
  autostartEnabled: boolean
  log: string[]
  canSendCommands: boolean
  appVersion: string
  appUpdateInfo: AppUpdateInfo | null
  appUpdateChecking: boolean
  appUpdateInstalling: boolean
  appUpdateError: string | null
  downloadProgress: UpdateDownloadProgress | null
  /** Wersja FW z handshake urządzenia (tylko odczyt). */
  deviceFw: string | null
  errorReportingEnabled: boolean
  onThemeChange: (t: 'system' | 'light' | 'dark') => void
  onLangChange: (l: Lang) => void
  onAutostartChange: (v: boolean) => void
  onErrorReportingChange: (v: boolean) => void
  onGetState: () => void
  onResetDefaults: () => void
  onCheckAppUpdate: () => void
  onInstallAppUpdate: () => void
  onClose: () => void
}

export function SettingsSheet({
  lang,
  theme,
  autostartEnabled,
  log,
  canSendCommands,
  appVersion,
  appUpdateInfo,
  appUpdateChecking,
  appUpdateInstalling,
  appUpdateError,
  downloadProgress,
  deviceFw,
  errorReportingEnabled,
  onThemeChange,
  onLangChange,
  onAutostartChange,
  onErrorReportingChange,
  onGetState,
  onResetDefaults,
  onCheckAppUpdate,
  onInstallAppUpdate,
  onClose,
}: Props) {
  const [confirmReset, setConfirmReset] = useState(false)
  const pl = lang === 'pl'
  const showAppBanner = appUpdateInfo?.available && appUpdateInfo.latest_version

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        background: 'var(--c-overlay)',
        display: 'flex',
        justifyContent: 'flex-end',
        zIndex: 150,
      }}
      onClick={onClose}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 340,
          background: 'var(--c-surface)',
          borderLeft: '0.5px solid var(--c-border)',
          height: '100%',
          padding: 20,
          overflowY: 'auto',
        }}
      >
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 20 }}>
          <div style={{ fontSize: 14, color: 'var(--c-text)' }}>{pl ? 'Ustawienia' : 'Settings'}</div>
          <button
            type="button"
            onClick={onClose}
            style={{ background: 'transparent', border: 'none', color: 'var(--c-text-muted)', cursor: 'pointer' }}
          >
            <X size={18} />
          </button>
        </div>

        <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, marginBottom: 8 }}>
          {pl ? 'AKTUALIZACJE PROGRAMU' : 'APP UPDATES'}
        </div>
        <div
          style={{
            padding: 12,
            borderRadius: 8,
            background: 'var(--c-seg)',
            border: '0.5px solid var(--c-border-soft)',
            marginBottom: 12,
          }}
        >
          <div style={{ fontSize: 12, color: 'var(--c-text-secondary)', marginBottom: 10 }}>
            {pl ? 'Aktualna wersja' : 'Current version'}:{' '}
            <span style={{ color: 'var(--c-text)', fontFamily: 'monospace' }}>v{appVersion}</span>
          </div>
          <button
            type="button"
            disabled={appUpdateChecking || appUpdateInstalling}
            onClick={onCheckAppUpdate}
            style={{
              width: '100%',
              background: 'var(--c-surface)',
              border: '0.5px solid var(--c-border-soft)',
              borderRadius: 6,
              padding: '8px 0',
              color: 'var(--c-text-secondary)',
              fontSize: 12,
              cursor: appUpdateChecking || appUpdateInstalling ? 'wait' : 'pointer',
              opacity: appUpdateChecking || appUpdateInstalling ? 0.6 : 1,
            }}
          >
            {appUpdateChecking
              ? pl
                ? 'Sprawdzanie…'
                : 'Checking…'
              : pl
                ? 'Sprawdź aktualizacje'
                : 'Check for updates'}
          </button>
          {appUpdateError && (
            <div style={{ fontSize: 11, color: 'var(--c-danger)', marginTop: 8 }} role="alert">
              {appUpdateError}
            </div>
          )}
          {!showAppBanner && appUpdateInfo && !appUpdateInfo.available && !appUpdateChecking && (
            <div style={{ fontSize: 11, color: 'var(--c-text-muted)', marginTop: 8 }}>
              {pl ? 'Masz najnowszą wersję aplikacji.' : 'You are on the latest app version.'}
            </div>
          )}
          {showAppBanner && (
            <div
              style={{
                marginTop: 12,
                padding: 10,
                borderRadius: 8,
                background: 'var(--c-accent-soft, rgba(99, 102, 241, 0.12))',
                border: '0.5px solid var(--c-border-soft)',
              }}
            >
              <div style={{ fontSize: 12, color: 'var(--c-text)', marginBottom: 4 }}>
                {pl ? 'Dostępna nowa wersja' : 'Update available'}:{' '}
                <strong>v{appUpdateInfo.latest_version}</strong>
              </div>
              {appUpdateInfo.body ? (
                <div
                  style={{
                    fontSize: 11,
                    color: 'var(--c-text-muted)',
                    marginBottom: 10,
                    whiteSpace: 'pre-wrap',
                    maxHeight: 120,
                    overflowY: 'auto',
                  }}
                >
                  {appUpdateInfo.body}
                </div>
              ) : null}
              <button
                type="button"
                disabled={appUpdateInstalling}
                onClick={onInstallAppUpdate}
                style={{
                  width: '100%',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap: 6,
                  background: 'var(--c-accent, #6366f1)',
                  border: 'none',
                  borderRadius: 6,
                  padding: '8px 0',
                  color: '#fff',
                  fontSize: 12,
                  cursor: appUpdateInstalling ? 'wait' : 'pointer',
                  opacity: appUpdateInstalling ? 0.7 : 1,
                }}
              >
                <Download size={14} />
                {appUpdateInstalling
                  ? pl
                    ? 'Pobieranie…'
                    : 'Downloading…'
                  : pl
                    ? 'Pobierz i zaktualizuj'
                    : 'Download and install'}
              </button>
              {downloadProgress && (appUpdateInstalling || downloadProgress.phase !== 'idle') && (
                <div style={{ marginTop: 10 }}>
                  <div
                    style={{
                      height: 4,
                      borderRadius: 2,
                      background: 'var(--c-border-soft)',
                      overflow: 'hidden',
                    }}
                  >
                    <div
                      style={{
                        height: '100%',
                        width: `${downloadProgress.percent ?? (downloadProgress.phase === 'finished' ? 100 : 8)}%`,
                        background: 'var(--c-accent, #6366f1)',
                        transition: 'width 0.15s ease',
                      }}
                    />
                  </div>
                  <div style={{ fontSize: 10, color: 'var(--c-text-dim)', marginTop: 4 }}>
                    {downloadProgress.percent != null
                      ? `${Math.round(downloadProgress.percent)}%`
                      : downloadProgress.phase}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>

        <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, margin: '8px 0 8px' }}>
          {pl ? 'URZĄDZENIE' : 'DEVICE'}
        </div>
        <div
          style={{
            padding: 12,
            borderRadius: 8,
            background: 'var(--c-seg)',
            border: '0.5px solid var(--c-border-soft)',
            marginBottom: 16,
            fontSize: 12,
            color: 'var(--c-text-secondary)',
          }}
        >
          {pl ? 'Firmware kontrolera' : 'Controller firmware'}:{' '}
          <span style={{ color: 'var(--c-text)', fontFamily: 'monospace' }}>
            {deviceFw ? `v${deviceFw}` : '—'}
          </span>
        </div>

        <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, marginBottom: 8 }}>APPEARANCE</div>
        <AssignmentRow icon={Monitor} label={pl ? 'Motyw' : 'Theme'}>
          <div style={{ display: 'flex', gap: 4, background: 'var(--c-seg)', borderRadius: 8, padding: 3 }}>
            {(['system', 'light', 'dark'] as const).map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => onThemeChange(t)}
                style={{
                  flex: 1,
                  fontSize: 11,
                  padding: '5px 0',
                  borderRadius: 6,
                  border: 'none',
                  cursor: 'pointer',
                  background: theme === t ? 'var(--c-seg-active)' : 'transparent',
                  color: theme === t ? 'var(--c-text)' : 'var(--c-text-muted)',
                }}
              >
                {t === 'system' ? 'System' : t === 'light' ? (pl ? 'Jasny' : 'Light') : pl ? 'Ciemny' : 'Dark'}
              </button>
            ))}
          </div>
        </AssignmentRow>
        <AssignmentRow icon={Radio} label={pl ? 'Język' : 'Language'}>
          <div style={{ display: 'flex', gap: 4, background: 'var(--c-seg)', borderRadius: 8, padding: 3, width: 100 }}>
            {(['pl', 'en'] as const).map((l) => (
              <button
                key={l}
                type="button"
                onClick={() => onLangChange(l)}
                style={{
                  flex: 1,
                  fontSize: 11,
                  padding: '5px 0',
                  borderRadius: 6,
                  border: 'none',
                  cursor: 'pointer',
                  background: lang === l ? 'var(--c-seg-active)' : 'transparent',
                  color: lang === l ? 'var(--c-text)' : 'var(--c-text-muted)',
                  textTransform: 'uppercase',
                }}
              >
                {l}
              </button>
            ))}
          </div>
        </AssignmentRow>

        <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, margin: '16px 0 8px' }}>STARTUP</div>
        <AssignmentRow icon={ChevronRight} label={pl ? 'Przy logowaniu' : 'Launch at login'}>
          <Toggle checked={autostartEnabled} onChange={onAutostartChange} />
        </AssignmentRow>

        <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, margin: '16px 0 8px' }}>
          {pl ? 'PRYWATNOŚĆ' : 'PRIVACY'}
        </div>
        <div
          style={{
            padding: 12,
            borderRadius: 8,
            background: 'var(--c-seg)',
            border: '0.5px solid var(--c-border-soft)',
            marginBottom: 8,
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, marginBottom: 8 }}>
            <div style={{ fontSize: 12, color: 'var(--c-text)' }}>
              {pl ? 'Anonimowe raportowanie błędów' : 'Anonymous error reporting'}
            </div>
            <Toggle checked={errorReportingEnabled} onChange={onErrorReportingChange} />
          </div>
          <div style={{ fontSize: 11, color: 'var(--c-text-dim)', lineHeight: 1.45 }}>
            {pl
              ? 'Automatycznie wysyłaj raporty o crashach i awariach do bazy idei.'
              : 'Automatically send crash and failure reports to the idei backend.'}
          </div>
        </div>

        <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, margin: '16px 0 8px' }}>HARDWARE</div>
        <div style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
          <button
            type="button"
            disabled={!canSendCommands}
            onClick={onGetState}
            style={{
              flex: 1,
              background: 'var(--c-seg)',
              border: '0.5px solid var(--c-border-soft)',
              borderRadius: 6,
              padding: '8px 0',
              color: 'var(--c-text-secondary)',
              fontSize: 12,
              cursor: 'pointer',
              opacity: canSendCommands ? 1 : 0.5,
            }}
          >
            {pl ? 'Pobierz stan' : 'Get state'}
          </button>
          <button
            type="button"
            disabled={!canSendCommands}
            onClick={() => {
              if (confirmReset) {
                onResetDefaults()
                setConfirmReset(false)
              } else {
                setConfirmReset(true)
              }
            }}
            style={{
              flex: 1,
              background: confirmReset ? 'var(--c-danger-bg)' : 'transparent',
              border: `0.5px solid ${confirmReset ? 'var(--c-danger)' : 'var(--c-danger-border-soft)'}`,
              borderRadius: 6,
              padding: '8px 0',
              color: 'var(--c-danger)',
              fontSize: 12,
              cursor: 'pointer',
              opacity: canSendCommands ? 1 : 0.5,
            }}
          >
            {confirmReset
              ? pl
                ? 'Kliknij aby potwierdzić'
                : 'Click to confirm'
              : pl
                ? 'Resetuj domyślne'
                : 'Reset to defaults'}
          </button>
        </div>
        {confirmReset && (
          <div style={{ fontSize: 10.5, color: 'var(--c-warn)', marginBottom: 8 }}>
            {pl
              ? 'To zgaśnie wszystkie diody LED na urządzeniu (Output Report 0x03).'
              : 'This turns off all device LEDs (Output Report 0x03).'}
          </div>
        )}

        <div style={{ fontSize: 11, color: 'var(--c-text-dim)', letterSpacing: 0.4, margin: '16px 0 8px' }}>EVENT LOG</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {log.slice(0, 30).map((line, i) => {
            const conflict = /conflict|konflikt/i.test(line)
            return (
              <div key={i} style={{ fontSize: 11, display: 'flex', gap: 8, color: 'var(--c-text-muted)', fontFamily: 'monospace' }}>
                <span style={{ color: conflict ? 'var(--c-warn)' : 'var(--c-text-muted)' }}>{line}</span>
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
