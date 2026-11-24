import { useEffect, useMemo } from 'react'
import { ArrowUpRight, Play, Power } from '@/ui/pixel-icons'
import { AsciiBar, Button, Caption } from '@/ui'
import { Box } from '@/boot/Boot'
import { DIRECT_DOWNLOAD, explainUpdateError, updateFraction, updates, useUpdates } from '@/lib/updates'
import { exitApp, openExternal, setStoreVisible } from '@/lib/window'
import { useLibrary } from '@/hooks/useLibrary'
import { playTarget, startEntry } from '@/lib/library'
import type { RequiredState } from '@/lib/required'

/**
 * This version is older than the oldest kryo.to allows (src-tauri required.rs):
 * the whole window, and it cannot be dismissed. Update now (the app's own
 * updater, progress in place, then a restart), Download it yourself when that
 * cannot work, Quit, and Play for games already installed: a forced update
 * must not hold installed games hostage when it cannot be fetched. Downloads,
 * uninstalling, moving, repairing and the Store are not here, and the native
 * side refuses the harmful ones from anywhere else too.
 */
export function RequiredUpdate({ required }: { required: RequiredState }) {
  const state = useUpdates()
  const lib = useLibrary()

  // The Store's page is a view of its own over the window: out of the way.
  useEffect(() => {
    void setStoreVisible(false)
    void updates.loadVersion()
    const s = updates.snapshot().status
    if (s === 'idle' || s === 'current' || s === 'failed') void updates.check()
  }, [])

  const games = useMemo(
    () => lib.games.filter((g) => !g.hidden).sort((a, b) => (b.lastPlayed ?? 0) - (a.lastPlayed ?? 0) || a.title.localeCompare(b.title)),
    [lib.games],
  )
  const working = state.status === 'downloading' || state.status === 'ready'
  const problem = state.status === 'failed' ? explainUpdateError(state.error) : null
  const update = () => (state.update ? updates.install() : updates.check().then(() => updates.install()))

  return (
    <Box>
      <div className="grid h-full overflow-y-auto">
        <div className="no-drag mx-auto grid w-full max-w-lg content-center gap-6 px-6 py-12">
          <header className="grid gap-2">
            <h1 className="text-lg font-bold text-foreground">Update Kryoto Desktop</h1>
            <p className="text-xs leading-relaxed text-foreground">
              {required.reason ?? 'This version has a serious problem, so it has to be updated before you go on.'}
            </p>
            <p className="text-[11px] text-muted-foreground">
              You have {required.current || state.current || 'an older version'}
              {required.min ? `; ${required.min} or newer fixes it.` : '.'}{' '}
              <button type="button" className="underline underline-offset-2 hover:text-foreground" onClick={() => void openExternal('https://kryo.to/changelog').catch(() => {})}>
                Why?
              </button>
            </p>
          </header>

          <section className="grid gap-3">
            {working ? (
              <div className="grid gap-1.5" role="status">
                <AsciiBar fraction={updateFraction(state)} cells={28} />
                <Caption>{state.status === 'ready' ? 'Restarting' : 'Downloading the update'}</Caption>
              </div>
            ) : (
              <Button variant="primary" size="lg" className="w-full" busy={state.status === 'checking'} busyLabel="Looking for the update" onClick={update}>
                {state.status === 'failed' ? 'Try again' : state.update ? `Update now to ${state.update.version}` : 'Update now'}
              </Button>
            )}
            {lib.running.size > 0 && !working ? (
              <p className="text-[11px] leading-relaxed text-muted-foreground">A game is running: close it first, the update restarts Kryoto Desktop.</p>
            ) : null}
            {state.status === 'current' ? (
              <p className="text-[11px] leading-relaxed text-muted-foreground">The update is not offered here yet. Download it yourself below, or try again in a few minutes.</p>
            ) : null}
            {problem ? (
              <p role="alert" className="text-[11px] leading-relaxed text-destructive">
                {problem.headline}
              </p>
            ) : state.status === 'failed' && state.error ? (
              <p role="alert" className="text-[11px] leading-relaxed text-destructive">
                {state.error}
              </p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <Button size="sm" onClick={() => openExternal(DIRECT_DOWNLOAD).catch(() => {})}>
                Download it yourself <ArrowUpRight className="size-3" aria-hidden />
              </Button>
              <Button size="sm" variant="ghost" onClick={() => exitApp()}>
                <Power className="size-3" aria-hidden /> Quit
              </Button>
            </div>
          </section>

          <section className="grid gap-2 border-t border-border pt-5">
            <h2 className="text-[11px] uppercase tracking-[0.2em] text-muted-foreground">Play meanwhile</h2>
            {games.length ? (
              <ul className="grid max-h-64 gap-1 overflow-y-auto">
                {games.map((g) => {
                  const running = lib.running.has(g.id)
                  const target = playTarget(g)
                  return (
                    <li key={g.id} className="kryo-radius flex items-center gap-3 border border-border px-3 py-1.5">
                      <span className="min-w-0 flex-1 truncate text-xs text-foreground">{g.title}</span>
                      <Button
                        size="sm"
                        variant={running ? 'ghost' : 'outline'}
                        disabled={running}
                        aria-label={running ? `${g.title} is running` : `Play ${g.title}`}
                        onClick={() => lib.play(g.id, target === 'ask' ? startEntry(g) : target)}
                      >
                        <Play className="size-3" aria-hidden /> {running ? 'Running' : 'Play'}
                      </Button>
                    </li>
                  )
                })}
              </ul>
            ) : (
              <p className="text-[11px] text-muted-foreground">{lib.loaded ? 'No games installed yet.' : 'Loading your library'}</p>
            )}
            {lib.error ? (
              <p role="alert" className="text-[11px] leading-relaxed text-destructive">
                {lib.error}
              </p>
            ) : null}
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              Downloads, uninstalling and moving games wait until Kryoto Desktop is updated.
            </p>
          </section>
        </div>
      </div>
    </Box>
  )
}
