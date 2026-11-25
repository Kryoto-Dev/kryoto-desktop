import { useCallback, useEffect, useMemo, useState } from 'react'
import { call, errorText, on } from '@/lib/bridge'
import { fetchCatalogGame, library } from '@/lib/library'
import { openExternal, openStorePage } from '@/lib/window'
import { ExternalLink, RotateCcw, TriangleAlert } from '@/ui/pixel-icons'
import { Busy, Button } from '@/ui'
import { RepairPanel, type RepairBackend, type RepairProgress } from './RepairPanel'
import { EmulatorRow } from './EmulatorRow'
import { OnlineFixRow } from './OnlineFixRow'
import { KryoFix } from './KryoFix'

/**
 * The auto-fixer for one installed game. With `slug`, kryo.to is asked first
 * whether its release is one the auto-fixer cannot fix (lib/autofix.ts): then
 * the release's notice shows instead, and nothing offers to repair it; only
 * Back to the release's own files stays, for what Kryoto changed before.
 * `onLeave` closes whatever holds this before its kryo.to page opens.
 * `view="row"`: Properties' Emulator row (EmulatorRow.tsx) and Online fix row
 * (OnlineFixRow.tsx) instead of the panel.
 */
export function GameRepair({ gameId, slug = null, initialOnline = false, purpose = 'repair', onLeave, view = 'panel', onGetFix }: { gameId: string; slug?: string | null; initialOnline?: boolean; purpose?: 'repair' | 'achievements'; onLeave?: () => void; view?: 'panel' | 'row'; /** Row view: kryo.to's fix needs the game's download window. */ onGetFix?: () => void }) {
  const backend = useMemo<RepairBackend>(() => ({
    sources: () => call('repair_sources'),
    state: () => call('repair_state', { gameId }),
    apply: (source) => call('repair_apply', { gameId, source }),
    undo: () => call('repair_undo', { gameId }),
    support: () => call('repair_support', { gameId }),
    openSupport: async () => { await openExternal(await call<string>('repair_support_url', { gameId })) },
    listen: (handler) => on<RepairProgress>('repair-progress', (progress) => { if (progress.gameId === gameId) handler(progress) }),
    openFolder: (path) => library.openFolder(path),
  }), [gameId])
  // Undefined while kryo.to is asked; null for a normal release (or no answer: the auto-fixer stays).
  const [blocked, setBlocked] = useState<{ notice: string; reason: string | null } | null | undefined>(slug ? undefined : null)
  useEffect(() => {
    if (!slug) return setBlocked(null)
    let cancelled = false
    setBlocked(undefined)
    // Not waited on for long: without an answer the auto-fixer is offered as before.
    const give = window.setTimeout(() => !cancelled && setBlocked((b) => (b === undefined ? null : b)), 4000)
    fetchCatalogGame(slug)
      .then((g) => !cancelled && setBlocked(g.noAutofix))
      .catch(() => !cancelled && setBlocked(null))
      .finally(() => window.clearTimeout(give))
    return () => {
      cancelled = true
      window.clearTimeout(give)
    }
  }, [slug])

  if (blocked === undefined) {
    return (
      <p role="status" className="flex items-center justify-center gap-2 py-6 text-xs text-muted-foreground">
        <Busy className="size-3.5" /> Checking this release
      </p>
    )
  }
  if (blocked) {
    const notice = (
      <section aria-label="Repair game" className="grid justify-items-center gap-3 text-center">
        <TriangleAlert className="size-6 text-warning" aria-hidden />
        <p className="max-w-sm text-xs leading-relaxed text-foreground">{blocked.notice}</p>
        {blocked.reason ? <p className="max-w-sm text-[11px] leading-relaxed text-foreground">Why: {blocked.reason}</p> : null}
        <p className="max-w-sm text-[11px] leading-relaxed text-muted-foreground">Kryoto&apos;s repair is off for this release. Still stuck? Ask on its kryo.to page or in support.</p>
        <BackToOwnFiles gameId={gameId} backend={backend} />
        {slug ? (
          <Button size="sm" variant="outline" onClick={() => { onLeave?.(); openStorePage(`/game/${slug}`) }}>
            <ExternalLink className="size-3.5" aria-hidden /> Its kryo.to page
          </Button>
        ) : null}
      </section>
    )
    // kryo.to's own fix, published by staff, stays offered as before; the
    // online-fix.me row goes with the auto-fixer.
    if (view === 'row' && slug) {
      return (
        <>
          {notice}
          <KryoFix gameId={gameId} slug={slug} onGet={onGetFix ?? (() => {})} />
        </>
      )
    }
    return notice
  }
  if (view === 'row') {
    return (
      <>
        <EmulatorRow gameId={gameId} backend={backend} />
        <OnlineFixRow gameId={gameId} slug={slug} backend={backend} onGet={onGetFix ?? (() => {})} />
      </>
    )
  }
  return <RepairPanel backend={backend} initialOnline={initialOnline} purpose={purpose} />
}

/**
 * On a release the auto-fixer is off for, the one way out that stays: files
 * Kryoto changed before kryo.to marked it (a repair, a switch, kryo.to's own
 * fix) go back to the release's own. A Phantom Blade Zero player repaired
 * before the mark could otherwise only reinstall. Nothing new goes in here.
 */
function BackToOwnFiles({ gameId, backend }: { gameId: string; backend: RepairBackend }) {
  const [undo, setUndo] = useState(false)
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const load = useCallback(() => {
    void library
      .emulators(gameId)
      .then((s) => setUndo(s.undo))
      .catch(() => setUndo(false))
  }, [gameId])
  useEffect(() => {
    load()
    let stop: (() => void) | undefined
    let cancelled = false
    // kryo.to's fix applied below, or Undo from elsewhere: read again.
    void on('library-changed', load).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [load])
  const restore = async () => {
    setBusy(true)
    setError(null)
    try {
      await backend.undo()
      setDone(true)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
      load()
    }
  }
  if (!undo && !done && !error) return null
  return (
    <div className="grid justify-items-center gap-2">
      {undo ? (
        <>
          <p className="max-w-sm text-[11px] leading-relaxed text-muted-foreground">Kryoto changed this game&apos;s files before. You can still put the release&apos;s own back.</p>
          <Button size="sm" variant="outline" disabled={busy} busy={busy} onClick={() => void restore()}>
            <RotateCcw className="size-3.5" aria-hidden /> Back to the release&apos;s own files
          </Button>
        </>
      ) : null}
      {done && !undo ? <p role="status" className="text-xs text-foreground">The release&apos;s own files are back.</p> : null}
      {error ? <p role="alert" className="max-w-sm break-words text-xs text-destructive">{error}</p> : null}
    </div>
  )
}
