import { RefreshCw } from 'lucide-react'
import type { DeviceInfo, Lang } from './types'

type Props = {
  lang: Lang
  devices: string[]
  selectedPath: string
  connectedDevices: Record<string, DeviceInfo>
  deviceLabels?: Record<string, string>
  scanning: boolean
  connecting: boolean
  isSelectedConnected: boolean
  canDisconnect: boolean
  onSelectDevice: (path: string) => void
  onConnect: () => void
  onDisconnect: () => void
  onScan: () => void
}

function shortLabel(path: string, info?: DeviceInfo, label?: string): string {
  if (label) return label
  if (info?.model) return `${info.model}${info.fw ? ` · fw ${info.fw}` : ''}`
  if (!path) return '—'
  const tail = path.split(/[\\/#]/).filter(Boolean).pop() ?? path
  return tail.length > 28 ? `…${tail.slice(-28)}` : tail
}

export function ConnectionPopover({
  lang,
  devices,
  selectedPath,
  connectedDevices,
  deviceLabels,
  scanning,
  connecting,
  isSelectedConnected,
  canDisconnect,
  onSelectDevice,
  onConnect,
  onDisconnect,
  onScan,
}: Props) {
  return (
    <div
      style={{
        position: 'absolute',
        top: 38,
        left: 0,
        background: 'var(--c-popover)',
        border: '0.5px solid var(--c-border-soft)',
        borderRadius: 10,
        padding: 14,
        width: 300,
        zIndex: 20,
        boxShadow: 'var(--c-shadow)',
      }}
      onClick={(e) => e.stopPropagation()}
    >
      <div style={{ fontSize: 11, color: 'var(--c-text-dim)', marginBottom: 6 }}>USB HID</div>
      <select
        value={selectedPath}
        onChange={(e) => onSelectDevice(e.target.value)}
        disabled={scanning}
        style={{
          width: '100%',
          background: 'var(--c-chip)',
          border: '0.5px solid var(--c-border-strong)',
          borderRadius: 6,
          padding: '7px 8px',
          color: 'var(--c-text)',
          fontSize: 12,
          marginBottom: 10,
        }}
      >
        {devices.length === 0 && !scanning && (
          <option value="">{lang === 'pl' ? '— brak —' : '— none found —'}</option>
        )}
        {scanning && devices.length === 0 && (
          <option value="">{lang === 'pl' ? 'Skanowanie…' : 'Scanning…'}</option>
        )}
        {devices.map((path) => {
          const info = connectedDevices[path]
          const label = shortLabel(path, info, deviceLabels?.[path])
          return (
            <option key={path} value={path}>
              {info ? `● ${label}` : label}
            </option>
          )
        })}
      </select>
      <div style={{ display: 'flex', gap: 8 }}>
        {isSelectedConnected ? (
          <button
            type="button"
            onClick={onDisconnect}
            disabled={!canDisconnect || connecting}
            style={{
              flex: 1,
              background: 'var(--c-danger-bg)',
              border: '0.5px solid var(--c-danger-border)',
              color: 'var(--c-danger)',
              borderRadius: 6,
              padding: '7px 0',
              fontSize: 12,
              cursor: 'pointer',
              opacity: !canDisconnect || connecting ? 0.5 : 1,
            }}
          >
            {lang === 'pl' ? 'Rozłącz' : 'Disconnect'}
          </button>
        ) : (
          <button
            type="button"
            onClick={onConnect}
            disabled={!selectedPath || connecting || scanning}
            style={{
              flex: 1,
              background: 'color-mix(in srgb, var(--c-accent) 12%, transparent)',
              border: '0.5px solid color-mix(in srgb, var(--c-accent) 33%, transparent)',
              color: 'var(--c-accent)',
              borderRadius: 6,
              padding: '7px 0',
              fontSize: 12,
              cursor: 'pointer',
              opacity: !selectedPath || connecting || scanning ? 0.5 : 1,
            }}
          >
            {connecting
              ? lang === 'pl'
                ? 'Łączenie…'
                : 'Connecting…'
              : lang === 'pl'
                ? 'Połącz'
                : 'Connect'}
          </button>
        )}
        <button
          type="button"
          onClick={onScan}
          disabled={scanning}
          style={{
            background: 'var(--c-chip)',
            border: '0.5px solid var(--c-border-strong)',
            borderRadius: 6,
            padding: '7px 10px',
            color: 'var(--c-text-secondary)',
            cursor: 'pointer',
            display: 'flex',
            alignItems: 'center',
            gap: 5,
            fontSize: 12,
            opacity: scanning ? 0.5 : 1,
          }}
        >
          <RefreshCw size={13} /> {lang === 'pl' ? 'Skanuj' : 'Scan'}
        </button>
      </div>
      <div style={{ fontSize: 10, color: 'var(--c-text-faint)', marginTop: 10 }}>
        {lang === 'pl'
          ? 'Auto-połączenie po wpięciu USB (VID 0483 / PID 5750).'
          : 'Auto-connects on USB plug (VID 0483 / PID 5750).'}
      </div>
    </div>
  )
}
