import { useEffect, useMemo, useRef, useState } from 'react'
import { SplashScene } from '@/boot/SplashScene'
import { on } from '@/lib/bridge'
import { useDownloads } from '@/lib/downloads'
import type { LibraryGame } from '@/lib/library'
import { dialogCount } from '@/ui/dialog-focus'

/**
 * Big Picture's screensaver (suggestion #107): after a few minutes with no
 * button, key, click or pointer, the start screen's own scene (SplashScene,
 * as Big Picture's intro and Kryoto Mobile use it) plays dimmed on black, with
 * one of your games named under the mark. Any input leaves it, and only wakes
 * the screen: that press does nothing else.
 *
 * Kept light and screen-safe: at most 20 frames a second, nothing drawn while
 * the window is hidden (KB-0012), and every 40 seconds the scene starts again
 * somewhere else on the screen, so nothing sits on the same pixels. Never
 * while a game runs, a question or menu is open, or a download has failed
 * since Big Picture opened. Settings > Devices > Controller, Big Picture,
 * turns it off or changes the wait.
 */

const ON_KEY = 'kryo:bp-screensaver'
const WAIT_KEY = 'kryo:bp-screensaver-minutes'
export const SCREENSAVER_WAITS = [2, 5, 10, 20] as const
const DEFAULT_WAIT = 5
/** A new place, and a new game, this often. */
const MOVE_MS = 40_000
/** How often the idle time is looked at. */
const CHECK_MS = 5_000

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}
function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    /* storage unavailable: the default again next time */
  }
}

/** On by default, like Big Picture's sounds. */
export function screensaverOn(): boolean {
  return read(ON_KEY) !== 'off'
}
export function setScreensaverOn(on: boolean) {
  write(ON_KEY, on ? 'on' : 'off')
}
/** Minutes without input before it starts. */
export function screensaverWait(): number {
  const n = Number(read(WAIT_KEY))
  return (SCREENSAVER_WAITS as readonly number[]).includes(n) ? n : DEFAULT_WAIT
}
export function setScreensaverWait(minutes: number) {
  write(WAIT_KEY, String(minutes))
}

/** Somewhere new on the screen, within a tenth of it from the middle. */
const place = () => ({ x: Math.round((Math.random() - 0.5) * 20), y: Math.round((Math.random() - 0.5) * 16) })

export function Screensaver({ games, running }: { games: LibraryGame[]; running: boolean }) {
  const [shown, setShown] = useState(false)
  const lastInput = useRef(Date.now())
  const { list: downloads } = useDownloads()
  // Failures already there when Big Picture opened were seen; a new one needs looking at.
  const failedBefore = useRef<Set<string> | null>(null)
  if (failedBefore.current === null && downloads.length) failedBefore.current = new Set(downloads.filter((d) => d.status === 'failed').map((d) => d.id))
  const newFailure = downloads.some((d) => d.status === 'failed' && !failedBefore.current?.has(d.id))
  const blocked = running || newFailure
  const blockedRef = useRef(blocked)
  blockedRef.current = blocked

  // Watching for input, and starting once it has been quiet long enough.
  useEffect(() => {
    if (!screensaverOn()) return
    const touched = () => (lastInput.current = Date.now())
    const events = ['keydown', 'pointerdown', 'pointermove', 'wheel', 'touchstart'] as const
    events.forEach((e) => window.addEventListener(e, touched, { capture: true, passive: true }))
    const stops: (() => void)[] = []
    let cancelled = false
    const keep = (fn: () => void) => (cancelled ? fn() : stops.push(fn))
    void on('pad-button', touched).then(keep)
    // A stick at rest still drifts a little: only a real push counts.
    void on<{ lx: number; ly: number; rx: number; ry: number }>('pad-axes', (a) => {
      if (Math.max(Math.abs(a.lx), Math.abs(a.ly), Math.abs(a.rx), Math.abs(a.ry)) > 0.35) touched()
    }).then(keep)
    const wait = screensaverWait() * 60_000
    const check = window.setInterval(() => {
      const busy = blockedRef.current || dialogCount() > 0 || !!document.querySelector('[data-kryo-float]') || document.hidden
      if (busy) lastInput.current = Date.now()
      else if (Date.now() - lastInput.current >= wait) setShown(true)
    }, CHECK_MS)
    return () => {
      cancelled = true
      events.forEach((e) => window.removeEventListener(e, touched, { capture: true }))
      stops.forEach((f) => f())
      window.clearInterval(check)
    }
  }, [])

  // Something that needs the player ends it at once.
  useEffect(() => {
    if (blocked) setShown(false)
  }, [blocked])

  if (!shown) return null
  return (
    <Saver
      games={games}
      onWake={() => {
        lastInput.current = Date.now()
        setShown(false)
      }}
    />
  )
}

function Saver({ games, onWake }: { games: LibraryGame[]; onWake: () => void }) {
  const box = useRef<HTMLDivElement | null>(null)
  const wake = useRef(onWake)
  wake.current = onWake
  // Games installed here, the ones played last first; none hidden or for adults.
  const names = useMemo(
    () =>
      games
        .filter((g) => !g.hidden && !g.nsfw)
        .sort((a, b) => (b.lastPlayed ?? 0) - (a.lastPlayed ?? 0))
        .slice(0, 24)
        .map((g) => g.title),
    [games],
  )
  const [turn, setTurn] = useState(0)
  const [at, setAt] = useState(place)
  const [label, setLabel] = useState(0)
  useEffect(() => {
    const t = window.setInterval(() => {
      setTurn((n) => n + 1)
      setAt(place())
    }, MOVE_MS)
    return () => window.clearInterval(t)
  }, [])

  // Any input wakes it, and that input goes nowhere else: Escape or B must
  // not also leave Big Picture, A must not also open a game.
  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null
    box.current?.focus({ preventScroll: true })
    const start = { x: -1, y: -1 }
    const swallow = (e: Event) => {
      e.preventDefault()
      e.stopImmediatePropagation()
      wake.current()
    }
    const moved = (e: PointerEvent) => {
      // A mouse that only settles is not someone coming back.
      if (start.x < 0) return void Object.assign(start, { x: e.clientX, y: e.clientY })
      if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) > 6) wake.current()
    }
    const keys = ['keydown', 'pointerdown', 'wheel'] as const
    const pad = ['kryo-pad-back', 'kryo-pad-confirm', 'kryo-pad-tab', 'kryo-pad-leave', 'kryo-pad-guide'] as const
    keys.forEach((k) => window.addEventListener(k, swallow, { capture: true }))
    pad.forEach((k) => window.addEventListener(k, swallow, { capture: true }))
    window.addEventListener('pointermove', moved, { capture: true })
    const stops: (() => void)[] = []
    let cancelled = false
    const keep = (fn: () => void) => (cancelled ? fn() : stops.push(fn))
    void on('pad-button', () => wake.current()).then(keep)
    void on<{ lx: number; ly: number; rx: number; ry: number }>('pad-axes', (a) => {
      if (Math.max(Math.abs(a.lx), Math.abs(a.ly), Math.abs(a.rx), Math.abs(a.ry)) > 0.5) wake.current()
    }).then(keep)
    return () => {
      cancelled = true
      keys.forEach((k) => window.removeEventListener(k, swallow, { capture: true }))
      pad.forEach((k) => window.removeEventListener(k, swallow, { capture: true }))
      window.removeEventListener('pointermove', moved, { capture: true })
      stops.forEach((f) => f())
      if (before?.isConnected) before.focus({ preventScroll: true })
    }
  }, [])

  const name = names.length ? names[turn % names.length] : 'Big Picture'
  return (
    <div
      ref={box}
      role="dialog"
      aria-modal="true"
      aria-label="Screensaver. Press any button to come back."
      tabIndex={-1}
      // A controller's A clicks what has focus (lib/pad-nav.ts): this.
      onClick={() => wake.current()}
      className="kryo-night kryo-fade fixed inset-0 z-[655] cursor-none overflow-hidden bg-black text-foreground outline-none"
    >
      <div className="absolute inset-0 opacity-45" style={{ transform: `translate(${at.x}vw, ${at.y}vh)` }}>
        <SplashScene key={turn} fps={20} fit={0.7} onMark={(bottom, cell) => setLabel(bottom + cell * 14)} onRevealed={() => {}} />
        {label ? (
          <p className="kryo-in pointer-events-none absolute inset-x-0 truncate px-8 pl-[0.6em] text-center text-sm font-bold uppercase tracking-[0.6em]" style={{ top: label }}>
            {name}
          </p>
        ) : null}
      </div>
    </div>
  )
}
