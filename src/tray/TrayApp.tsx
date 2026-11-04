import { useEffect } from 'react'
import { Download, Gamepad2, Library, LogOut, Settings, Store, X } from 'lucide-react'
import { call } from '@/lib/bridge'

type TrayAction = 'open' | 'store' | 'library' | 'downloads' | 'settings' | 'exit'

const actions: { id: Exclude<TrayAction, 'open' | 'exit'>; label: string; detail: string; icon: typeof Store }[] = [
  { id: 'store', label: 'Store', detail: 'Browse new worlds', icon: Store },
  { id: 'library', label: 'Library', detail: 'Your installed games', icon: Library },
  { id: 'downloads', label: 'Downloads', detail: 'Installs and updates', icon: Download },
  { id: 'settings', label: 'Settings', detail: 'Tune your launcher', icon: Settings },
]

export function TrayApp() {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') void call('tray_action', { action: 'open' })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const choose = (action: TrayAction) => void call('tray_action', { action })

  return (
    <main className="tray-panel" role="menu" aria-label="Kryoto quick menu">
      <header className="tray-header">
        <div className="tray-brand">
          <span className="tray-mark"><Gamepad2 aria-hidden /></span>
          <span><strong>KRYOTO</strong><small>DESKTOP</small></span>
        </div>
        <button type="button" className="tray-close" aria-label="Close menu" onClick={() => choose('open')}>
          <X aria-hidden />
        </button>
      </header>
      <button type="button" className="tray-hero" role="menuitem" onClick={() => choose('open')}>
        <span className="tray-hero-copy"><small>BACK TO YOUR DESK</small><strong>Open Kryoto</strong><span>Pick up where you left off.</span></span>
        <span className="tray-hero-arrow">↗</span>
      </button>
      <div className="tray-section-label">Jump to</div>
      <div className="tray-actions">
        {actions.map(({ id, label, detail, icon: Icon }) => (
          <button key={id} type="button" role="menuitem" className="tray-action" onClick={() => choose(id)}>
            <span className="tray-action-icon"><Icon aria-hidden /></span>
            <span><strong>{label}</strong><small>{detail}</small></span>
            <span className="tray-action-chevron">›</span>
          </button>
        ))}
      </div>
      <footer className="tray-footer">
        <button type="button" role="menuitem" onClick={() => choose('exit')}><LogOut aria-hidden /> Exit Kryoto</button>
        <span>ESC to close</span>
      </footer>
    </main>
  )
}
