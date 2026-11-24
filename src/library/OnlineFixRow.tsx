import { useCallback, useEffect, useRef, useState } from 'react'
import { Download, ExternalLink, FileArchive, RotateCcw, RotateCw } from '@/ui/pixel-icons'
import { Busy, Button, Section } from '@/ui'
import { errorText, on } from '@/lib/bridge'
import { library } from '@/lib/library'
import { ONLINE_FIX_HINT, onlineFixView, type OnlineFixState } from '@/lib/online-fix'
import { pickPath } from '@/lib/pick'
import type { RepairBackend, RepairProgress } from './RepairPanel'
import { KryoFix } from './KryoFix'

/**
 * Properties > Emulator & fixes > Online fix (suggestion #91): Online-Fix's
 * fix for this game, in plain words, and one press to get it.
 *
 * kryo.to's own packed copy comes first when staff published one (KryoFix,
 * the same component as before). Otherwise online-fix.me is asked what it has
 * (src-tauri online_fix.rs), and "Get the online fix" opens its folder in a
 * window where the player clicks the file; Kryoto checks it and puts it in
 * with a copy of every file it replaces. "I have the file" takes an archive
 * the player downloaded themselves. Removing it is Back to the release's own
 * files, the same Undo as for an emulator. Hidden with the auto-fixer when a
 * release blocks it (GameRepair.tsx, lib/autofix.ts).
 */
export function OnlineFixRow({ gameId, slug, backend, onGet }: { gameId: string; slug: string | null; backend: RepairBackend; onGet: () => void }) {
  const [state, setState] = useState<OnlineFixState | null>(null)
  const [busy, setBusy] = useState<'get' | 'file' | 'look' | 'remove' | null>(null)
  const [status, setStatus] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<string | null>(null)
  const inFlight = useRef(false)

  const look = useCallback(
    (fresh: boolean) =>
      library
        .onlineFixLookup(gameId, fresh)
        .then(setState)
        .catch((e: unknown) => setError(errorText(e))),
    [gameId],
  )
  useEffect(() => {
    let cancelled = false
    setState(null)
    // What is known at once, then online-fix.me asked (once a run per game).
    void library
      .onlineFixState(gameId)
      .then((s) => {
        if (cancelled) return
        setState(s)
        if (!s.blocked && !s.lookup) void look(false)
      })
      .catch((e: unknown) => setError(errorText(e)))
    return () => {
      cancelled = true
    }
  }, [gameId, look])
  useEffect(() => {
    const stops: (() => void)[] = []
    let cancelled = false
    const keep = (fn: () => void) => (cancelled ? fn() : stops.push(fn))
    void on<RepairProgress>('repair-progress', (p) => {
      if (p.gameId === gameId && inFlight.current) setStatus(p.text)
    }).then(keep)
    // An emulator switch or Back (the Emulator row, above) changes what is in.
    void on('library-changed', () => {
      if (!inFlight.current) void library.onlineFixState(gameId).then(setState).catch(() => {})
    }).then(keep)
    return () => {
      cancelled = true
      stops.forEach((f) => f())
    }
  }, [gameId])

  const act = async (kind: 'get' | 'file' | 'look' | 'remove', work: () => Promise<OnlineFixState | void>, after: string | null) => {
    if (inFlight.current) return
    inFlight.current = true
    setBusy(kind)
    setError(null)
    setDone(null)
    setStatus(null)
    try {
      const next = await work()
      if (next) setState(next)
      else setState(await library.onlineFixState(gameId))
      setDone(after)
    } catch (e) {
      setError(errorText(e))
    } finally {
      inFlight.current = false
      setBusy(null)
      setStatus(null)
    }
  }
  const DONE = 'Done. Start the game with Steam running. Friends who play with you need the same fix and game version.'
  const get = () => act('get', () => library.onlineFixGet(gameId), DONE)
  const fromFile = async () => {
    const path = await pickPath({ title: "Pick the online fix's archive", extensions: ['rar', 'zip', '7z'], filterName: 'Online fix' }).catch(() => null)
    if (path) await act('file', () => library.onlineFixFromFile(gameId, path), DONE)
  }
  const remove = () => act('remove', () => backend.undo(), "The online fix is out. The release's own files are back.")

  if (state?.blocked) return null
  const view = state ? onlineFixView(state) : null

  return (
    <Section title="Online fix" hint={ONLINE_FIX_HINT}>
      {slug ? <KryoFix gameId={gameId} slug={slug} onGet={onGet} bare /> : null}
      {!view ? (
        error ? null : (
          <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground">
            <Busy className="size-3.5" /> Looking on online-fix.me
          </p>
        )
      ) : (
        <div className="grid gap-1">
          <p className="text-xs text-foreground">{view.line}</p>
          {view.version ? <p className={`text-[11px] ${state?.lookup?.newerNeeded ? 'text-warning' : 'text-muted-foreground'}`}>{view.version}</p> : null}
          {view.get && state && !state.sevenZip ? (
            <p className="text-[11px] text-muted-foreground">Opening it needs 7-Zip (7-zip.org) on this PC.</p>
          ) : null}
        </div>
      )}
      {busy && status ? (
        <p role="status" aria-live="polite" className="text-xs text-muted-foreground">
          {status}
        </p>
      ) : null}
      {done ? (
        <p role="status" className="text-xs text-foreground">
          {done}
        </p>
      ) : null}
      {view ? (
        <div className="flex flex-wrap gap-2">
          {view.get ? (
            <Button size="sm" variant={view.get === 'primary' ? 'primary' : 'outline'} disabled={!!busy} busy={busy === 'get'} onClick={() => void get()}>
              <Download className="size-3.5" aria-hidden /> {view.get === 'primary' ? 'Get the online fix' : 'Get its files again'}
            </Button>
          ) : null}
          {view.fromFile ? (
            <Button size="sm" variant="ghost" disabled={!!busy} busy={busy === 'file'} onClick={() => void fromFile()}>
              <FileArchive className="size-3.5" aria-hidden /> I have the file
            </Button>
          ) : null}
          {view.openPage ? (
            <Button size="sm" variant="ghost" onClick={() => void library.onlineFixOpenPage(gameId).catch((e: unknown) => setError(errorText(e)))}>
              <ExternalLink className="size-3.5" aria-hidden /> Open online-fix.me
            </Button>
          ) : null}
          {view.retry ? (
            <Button size="sm" variant="ghost" disabled={!!busy} busy={busy === 'look'} onClick={() => void act('look', () => look(true), null)}>
              <RotateCw className="size-3.5" aria-hidden /> Look again
            </Button>
          ) : null}
          {view.remove ? (
            <Button size="sm" variant="ghost" disabled={!!busy} busy={busy === 'remove'} onClick={() => void remove()}>
              <RotateCcw className="size-3.5" aria-hidden /> Remove the online fix
            </Button>
          ) : null}
        </div>
      ) : null}
      {error ? (
        <p role="alert" className="break-words text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </Section>
  )
}
