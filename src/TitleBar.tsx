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
      className="h-10 shrink-0 flex items-center justify-between pl-4 pr-1 bg-slate-200/95 border-b border-slate-300 select-none rounded-t-xl dark:bg-slate-800/90 dark:border-slate-700/60 cursor-grab active:cursor-grabbing"
      onContextMenu={stopNativeBehavior}
      onDoubleClick={stopNativeBehavior}
      onMouseDown={handleTitleBarMouseDown}
    >
      <span
        data-tauri-drag-region
        className="flex-1 min-w-0 text-sm font-medium text-slate-700 dark:text-slate-300"
      >
        IDEI Control
      </span>
      <div className="titlebar-no-drag flex items-center shrink-0">
        <button
          type="button"
          onClick={() => { win.minimize().catch(() => {}) }}
          className="w-10 h-10 flex items-center justify-center text-slate-500 hover:bg-slate-300/80 hover:text-slate-800 rounded transition-colors dark:text-slate-400 dark:hover:bg-slate-700/80 dark:hover:text-slate-200"
          aria-label="Minimize"
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden>
            <rect y="5" width="12" height="1" rx="0.5" />
          </svg>
        </button>
        <button
          type="button"
          onClick={() => { win.toggleMaximize().catch(() => {}) }}
          className="w-10 h-10 flex items-center justify-center text-slate-500 hover:bg-slate-300/80 hover:text-slate-800 rounded transition-colors dark:text-slate-400 dark:hover:bg-slate-700/80 dark:hover:text-slate-200"
          aria-label="Maximize"
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden>
            <rect x="0.5" y="0.5" width="11" height="11" rx="1" stroke="currentColor" strokeWidth="1" fill="none" />
          </svg>
        </button>
        <button
          type="button"
          onClick={() => { win.close().catch(() => {}) }}
          className="w-10 h-10 flex items-center justify-center text-slate-500 hover:bg-red-500/90 hover:text-white rounded transition-colors dark:text-slate-400 dark:hover:bg-red-600/80"
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
