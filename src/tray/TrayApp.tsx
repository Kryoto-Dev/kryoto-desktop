import { useEffect, useMemo, useState } from 'react'
import { Download, Library, LogOut, Settings, Store, X } from 'lucide-react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { emitTo } from '@tauri-apps/api/event'
import { call, errorText } from '@/lib/bridge'
import { KryoMark } from '@/ui/ascii/KryoMark'
import { useLibrary } from '@/hooks/useLibrary'
import { artSrc } from '@/lib/art'
import { coverFor } from '@/lib/game-artwork'
import { isCustomArt } from '@/lib/game-artwork'

type TrayAction = 'open' | 'store' | 'library' | 'downloads' | 'settings' | 'exit'

const actions: { id: Exclude<TrayAction, 'open' | 'exit'>; label: string; hint: string; icon: typeof Store }[] = [
  { id: 'store', label: 'Store', hint: 'Browse', icon: Store },
  { id: 'library', label: 'Library', hint: 'Play', icon: Library },
  { id: 'downloads', label: 'Downloads', hint: 'Updates', icon: Download },
  { id: 'settings', label: 'Settings', hint: 'Configure', icon: Settings },
]

export function TrayApp() {
  const { games } = useLibrary()
  const [actionError, setActionError] = useState<string | null>(null)
  const [brokenArt, setBrokenArt] = useState<Set<string>>(new Set())
  const recent = useMemo(
    () => [...games].sort((a, b) => (b.lastPlayed ?? b.addedAt) - (a.lastPlayed ?? a.addedAt)).slice(0, 3),
    [games],
  )

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') void call('tray_action', { action: 'open' })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const choose = async (action: TrayAction, gameId?: string) => {
    setActionError(null)
    try {
      if (action !== 'open' && action !== 'exit') {
        await emitTo('main', 'tray-go', gameId ? { action: 'game', gameId } : action)
      }
      await call('tray_action', { action: action === 'exit' ? 'exit' : 'open' })
    } catch (error) {
      setActionError(errorText(error))
      await getCurrentWindow().show().catch(() => {})
    }
  }

  return (
    <main className="tray-panel" role="menu" aria-label="Kryoto quick menu">
      <header className="tray-header">
        <div className="tray-brand">
          <span className="tray-mark"><KryoMark className="h-3.5 text-primary-foreground" /></span>
          <span><strong>KRYOTO</strong><small>DESKTOP</small></span>
        </div>
        <button type="button" className="tray-close" aria-label="Close menu" onClick={() => choose('open')}>
          <X aria-hidden />
        </button>
      </header>
      <div className="tray-section-label">Quick access</div>
      <div className="tray-actions">
        {actions.map(({ id, label, hint, icon: Icon }) => (
          <button key={id} type="button" role="menuitem" className="tray-action" onClick={() => choose(id)}>
            <span className="tray-action-icon"><Icon aria-hidden /></span>
            <strong>{label}</strong>
            <small>{hint}</small>
            <span className="tray-action-chevron">›</span>
          </button>
        ))}
      </div>
      {recent.length > 0 ? (
        <section className="tray-recent" aria-label="Recent games">
          <div className="tray-section-label">Recent games</div>
          <div className="tray-games">
            {recent.map((game) => {
              const cover = coverFor(game)
              const image = cover && (isCustomArt(cover) ? artSrc(cover) : cover)
              return (
                <button key={game.id} type="button" className="tray-game" onClick={() => choose('library', game.id)} title={`Open ${game.title}`}>
                  {image && !brokenArt.has(image) ? (
                    <img
                      src={image}
                      alt=""
                      onError={() => setBrokenArt((current) => new Set(current).add(image))}
                    />
                  ) : (
                    <span className="tray-game-empty"><KryoMark className="h-3.5" /></span>
                  )}
                  <span>{game.title}</span>
                </button>
              )
            })}
          </div>
        </section>
      ) : null}
      {actionError ? <p className="tray-error" role="alert">{actionError}</p> : null}
      <footer className="tray-footer">
        <button type="button" role="menuitem" onClick={() => choose('exit')}><LogOut aria-hidden /> Exit Kryoto</button>
        <span>ESC close</span>
      </footer>
    </main>
  )
}
