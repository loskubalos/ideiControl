import { getCurrentWindow } from '@tauri-apps/api/window'

export function TitleBar() {
  const win = getCurrentWindow()

  const stopNativeBehavior = (e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
  }

  const handleTitleBarMouseDown = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('button')) return
    if (e.detail === 2) {
      e.preventDefault()
      e.stopPropagation()
      return
    }
    if (e.button === 0 && e.detail === 1) {
      win.startDragging().catch(() => {})
    }
  }

  return (
    <div
      data-tauri-drag-region
      className="h-10 shrink-0 flex items-center justify-between pl-4 pr-1 select-none rounded-t-xl cursor-grab active:cursor-grabbing titlebar-console"
      onContextMenu={stopNativeBehavior}
      onDoubleClick={stopNativeBehavior}
      onMouseDown={handleTitleBarMouseDown}
    >
      <span data-tauri-drag-region className="flex-1 min-w-0 text-sm font-medium">
        IDEI Control
      </span>
      <div className="titlebar-no-drag flex items-center shrink-0">
        <button
          type="button"
          onClick={() => {
            win.minimize().catch(() => {})
          }}
          className="w-10 h-10 flex items-center justify-center rounded transition-colors"
          aria-label="Minimize"
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden>
            <rect y="5" width="12" height="1" rx="0.5" />
          </svg>
        </button>
        <button
          type="button"
          onClick={() => {
            win.toggleMaximize().catch(() => {})
          }}
          className="w-10 h-10 flex items-center justify-center rounded transition-colors"
          aria-label="Maximize"
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden>
            <rect x="0.5" y="0.5" width="11" height="11" rx="1" stroke="currentColor" strokeWidth="1" fill="none" />
          </svg>
        </button>
        <button
          type="button"
          onClick={() => {
            win.close().catch(() => {})
          }}
          className="w-10 h-10 flex items-center justify-center rounded transition-colors hover:bg-red-600/80 hover:text-white"
          aria-label="Close"
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden>
            <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
          </svg>
        </button>
      </div>
    </div>
  )
}
