import { useCallback, useEffect, useRef, useState } from 'react'
import { Check, LifeBuoy, RotateCcw, X } from '@/ui/pixel-icons'
import { Busy, Button, Section } from '@/ui'
import { errorText, on } from '@/lib/bridge'
import { library, type GameEmulators } from '@/lib/library'
import type { RepairBackend, RepairProgress } from './RepairPanel'

/**
 * Properties > Emulator: which Steam emulator the game uses now, in plain
 * words, and every other one that fits it, one press each, with what each is
 * good for. The owner asked for switching to be easy to find, not hidden
 * behind Repair (2026-11-22).
 *
 * Everything comes from one call (repair.rs `game_emulators`), and a switch is
 * the existing engine (`repair_switch`: Undo of what Kryoto put in, then
 * Repair's `apply`, which keeps a copy of every file it replaces). Progress
 * shows in place; after it, Working / Not working, and Back to the release's
 * own files whenever Kryoto has a copy.
 */
export function EmulatorRow({ gameId, backend }: { gameId: string; backend: RepairBackend }) {
  const [state, setState] = useState<GameEmulators | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [status, setStatus] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<string | null>(null)
  const [answer, setAnswer] = useState<'working' | 'broken' | null>(null)
  const inFlight = useRef(false)

  const load = useCallback(() => {
    void library
      .emulators(gameId)
      .then(setState)
      .catch((e: unknown) => setError(errorText(e)))
  }, [gameId])
  useEffect(load, [load])
  useEffect(() => {
    const stops: (() => void)[] = []
    let cancelled = false
    const keep = (fn: () => void) => (cancelled ? fn() : stops.push(fn))
    void on<RepairProgress>('repair-progress', (p) => {
      if (p.gameId === gameId) setStatus(p.text + (p.percent != null ? ` ${p.percent}%` : ''))
    }).then(keep)
    // A fix from kryo.to applied below, or a switch made elsewhere: read again.
    void on('library-changed', () => { if (!inFlight.current) load() }).then(keep)
    return () => {
      cancelled = true
      stops.forEach((f) => f())
    }
  }, [gameId, load])

  const act = async (label: string, work: () => Promise<unknown>, after: string) => {
    if (inFlight.current) return
    inFlight.current = true
    setBusy(label)
    setError(null)
    setDone(null)
    setAnswer(null)
    setStatus(null)
    try {
      await work()
      setDone(after)
    } catch (e) {
      setError(errorText(e))
    } finally {
      inFlight.current = false
      setBusy(null)
      setStatus(null)
      load()
    }
  }
  const switchTo = (id: string, label: string) =>
    act(label, () => library.switchEmulator(gameId, id), `Now using ${label}. Start the game to check that it works.`)
  const restore = () => act('restore', () => backend.undo(), "The release's own files are back.")
  const report = async () => {
    setAnswer('broken')
    try {
      await backend.support()
      setDone('Your report was copied. Contact support and paste it (Ctrl+V).')
    } catch (e) {
      setError(errorText(e))
    }
  }

  if (!state) {
    return (
      <Section title="Emulator">
        {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground"><Busy className="size-3.5" /> Looking at the game's files</p>}
      </Section>
    )
  }

  const now = state.current ?? 'Not known'
  return (
    <Section
      title="Emulator"
      hint="What lets the game run without Steam's own login. Close the game first. Kryoto keeps a copy of every file it replaces, so you can always go back."
    >
      <div className="grid gap-1">
        <p className="text-sm text-foreground">
          Now: <b>{now}</b>
        </p>
        {state.switched ? (
          <p className="text-[11px] text-muted-foreground">
            Put in by Kryoto. {state.original ? `The release came with ${state.original.replace(/^Steam \+ /, '')}.` : 'Back puts the release\'s own files back.'}
          </p>
        ) : null}
      </div>
      {state.interrupted ? <p role="alert" className="text-xs text-destructive">A switch stopped halfway. Go back to the release&apos;s own files first.</p> : null}
      {state.blocked ? (
        <p className="text-xs text-muted-foreground">{state.blocked}</p>
      ) : (
        <ul className="grid gap-2" aria-label="Switch to">
          {state.choices.map((c) => (
            <li key={c.id} className="kryo-radius flex items-center gap-3 border border-border p-3">
              <span className="grid min-w-0 grow gap-0.5">
                <span className="text-xs font-bold text-foreground">{c.label}</span>
                <span className="text-[11px] leading-snug text-muted-foreground">{c.purpose}</span>
              </span>
              <Button size="sm" variant="outline" disabled={!!busy || state.interrupted} busy={busy === c.label} onClick={() => void switchTo(c.id, c.label)}>
                Switch
              </Button>
            </li>
          ))}
        </ul>
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
      {done && state.switched && !answer && !busy ? (
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={() => { setAnswer('working'); setDone('Good. The copy of the old files stays, in case you want to go back.') }}>
            <Check className="size-3.5" aria-hidden /> Working
          </Button>
          <Button size="sm" variant="outline" onClick={() => void report()}>
            <X className="size-3.5" aria-hidden /> Not working
          </Button>
        </div>
      ) : null}
      <div className="flex flex-wrap gap-2">
        {state.undo ? (
          <Button size="sm" variant="ghost" disabled={!!busy} busy={busy === 'restore'} onClick={() => void restore()}>
            <RotateCcw className="size-3.5" aria-hidden /> Back to the release&apos;s own files
          </Button>
        ) : null}
        {answer === 'broken' || error ? (
          <Button size="sm" variant="ghost" onClick={() => backend.openSupport().catch((e: unknown) => setError(errorText(e)))}>
            <LifeBuoy className="size-3.5" aria-hidden /> Contact support
          </Button>
        ) : null}
      </div>
      {error ? <p role="alert" className="break-words text-xs text-destructive">{error}</p> : null}
    </Section>
  )
}
