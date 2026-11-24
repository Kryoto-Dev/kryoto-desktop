import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { on } from '@/lib/bridge'
import { useDownloads } from '@/lib/downloads'
import type { LibraryGame } from '@/lib/library'
import { useHideAdult, useShowAdult } from '@/lib/adult'
import { dialogCount } from '@/ui/dialog-focus'
import { DEFAULT_SAVER, SAVER_IDS, type SaverId } from './screensavers'
import { SaverCanvas } from './screensavers/SaverCanvas'
import { lastPlayedTitle, saverTitles } from './screensavers/titles'

/**
 * Big Picture's screensaver (suggestion #107): after a few minutes with no
 * button, key, click or pointer, one of Kryoto's screensavers plays on black
 * (screensavers/: Mark, the default, Clock, Field, Rain, Snake, or all of
 * them in turn). Any input leaves it, and only wakes the screen: that press
 * does nothing else.
 *
 * Kept light and screen-safe: at most 20 frames a second, nothing drawn while
 * the window is hidden (KB-0012), and everything lit keeps moving, so nothing
 * sits on the same pixels. Never while a game runs, a question or menu is
 * open, or a download has failed since Big Picture opened. Settings > Devices >
 * Controller, Big Picture, picks one, turns it off, changes the wait, and
 * shows it at once.
 */

const ON_KEY = 'kryo:bp-screensaver'
const WAIT_KEY = 'kryo:bp-screensaver-minutes'
const WHICH_KEY = 'kryo:bp-screensaver-variant'
export const SCREENSAVER_WAITS = [2, 5, 10, 20] as const
const DEFAULT_WAIT = 5
/** Full screen, at most this many frames a second (KB-0012). */
const FPS = 20
/** How often the idle time is looked at. */
const CHECK_MS = 5_000
/** Input this soon after it starts is the press that started it ("Show it now"), not someone coming back. */
const SETTLE_MS = 600

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
/** Which one plays: Mark unless another was picked. */
export function screensaverVariant(): SaverId {
  const v = read(WHICH_KEY)
  return (SAVER_IDS as readonly string[]).includes(v ?? '') ? (v as SaverId) : DEFAULT_SAVER
}
export function setScreensaverVariant(id: SaverId) {
  write(WHICH_KEY, id)
}

/** The titles a screensaver may show, and the one played last: none hidden, adult only if shown. */
export function useSaverTitles(games: readonly LibraryGame[]) {
  const show = useShowAdult()
  const hide = useHideAdult()
  return useMemo(
    () => ({ titles: saverTitles(games, { show, hide }), lastPlayed: lastPlayedTitle(games, { show, hide }) }),
    [games, show, hide],
  )
}

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

/**
 * The screensaver itself, full screen over everything, until any input.
 * Big Picture shows it when left alone; Settings' "Show it now" shows it at
 * once, with the one picked there.
 */
export function Saver({ games, onWake, variant }: { games: readonly LibraryGame[]; onWake: () => void; variant?: SaverId }) {
  const box = useRef<HTMLDivElement | null>(null)
  const wake = useRef(onWake)
  wake.current = onWake
  const { titles, lastPlayed } = useSaverTitles(games)
  const [id] = useState<SaverId>(() => variant ?? screensaverVariant())

  // Any input wakes it, and that input goes nowhere else: Escape or B must
  // not also leave Big Picture, A must not also open a game.
  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null
    box.current?.focus({ preventScroll: true })
    const opened = performance.now()
    const settled = () => performance.now() - opened > SETTLE_MS
    const start = { x: -1, y: -1 }
    const swallow = (e: Event) => {
      e.preventDefault()
      e.stopImmediatePropagation()
      if (settled()) wake.current()
    }
    const moved = (e: PointerEvent) => {
      // A mouse that only settles is not someone coming back.
      if (start.x < 0 || !settled()) return void Object.assign(start, { x: e.clientX, y: e.clientY })
      if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) > 6) wake.current()
    }
    const keys = ['keydown', 'pointerdown', 'wheel'] as const
    const pad = ['kryo-pad-back', 'kryo-pad-confirm', 'kryo-pad-tab', 'kryo-pad-leave', 'kryo-pad-guide'] as const
    // Not passive: the wheel that wakes it must not also scroll what is behind.
    keys.forEach((k) => window.addEventListener(k, swallow, { capture: true, passive: false }))
    pad.forEach((k) => window.addEventListener(k, swallow, { capture: true }))
    window.addEventListener('pointermove', moved, { capture: true })
    const stops: (() => void)[] = []
    let cancelled = false
    const keep = (fn: () => void) => (cancelled ? fn() : stops.push(fn))
    void on('pad-button', () => settled() && wake.current()).then(keep)
    void on<{ lx: number; ly: number; rx: number; ry: number }>('pad-axes', (a) => {
      if (settled() && Math.max(Math.abs(a.lx), Math.abs(a.ly), Math.abs(a.rx), Math.abs(a.ry)) > 0.5) wake.current()
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

  // On the body: a page that moves (Settings slides in) would otherwise hold it inside its own box.
  return createPortal(
    <div
      ref={box}
      role="dialog"
      aria-modal="true"
      aria-label="Screensaver. Press any button to come back."
      tabIndex={-1}
      data-screensaver={id}
      // A controller's A clicks what has focus (lib/pad-nav.ts): this.
      onClick={() => wake.current()}
      className="kryo-night kryo-fade fixed inset-0 z-[950] cursor-none overflow-hidden bg-black text-foreground outline-none"
    >
      <SaverCanvas id={id} fps={FPS} titles={titles} lastPlayed={lastPlayed} />
    </div>,
    document.body,
  )
}
