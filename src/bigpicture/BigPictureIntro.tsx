import { useEffect, useRef, useState } from 'react'
import { SplashScene } from '@/boot/SplashScene'
import { play } from '@/lib/sound'

/**
 * Big Picture's opening: Kryoto Desktop's own start screen (SplashScene, the
 * same one Kryoto Mobile opens with), full screen. The game cases spiral in
 * and light the K//, BIG PICTURE comes in under it where the start box names
 * the app, and it gives way to the library. Any button, key or click skips
 * it; with reduced motion it moves straight on.
 */

/** The start box the scene was drawn for (lib/window.ts, `splash`). */
const BOX_W = 340
const BOX_H = 380
/** How long BIG PICTURE holds once the mark is lit, then the fade. */
const HOLD_MS = 650
const FADE_MS = 350
/** The scene lights the mark this long after it starts (SplashScene's burst). */
const LIGHTS_AT_MS = 1745
/** Where the sound's chord sits (sound-recipes.ts, `bpIntro`). */
const CHORD_AT_MS = 900

export function reducedMotion() {
  return typeof window !== 'undefined' && (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches || document.documentElement.dataset.motion === 'reduced')
}

export function BigPictureIntro({ onDone }: { onDone: () => void }) {
  const [lit, setLit] = useState(false)
  const [leaving, setLeaving] = useState(false)
  const done = useRef(false)
  const finish = useRef(() => {})
  finish.current = () => {
    if (done.current) return
    done.current = true
    onDone()
  }
  // Big enough to fill the screen the way it fills the start box.
  const [cell] = useState(() => Math.max(1, Math.min(window.innerWidth / BOX_W, window.innerHeight / BOX_H) * 0.85))

  useEffect(() => {
    if (reducedMotion()) {
      finish.current()
      return
    }
    // The chord lands as the mark lights.
    const sound = window.setTimeout(() => play('bpIntro', { bigPicture: true }), LIGHTS_AT_MS - CHORD_AT_MS)
    const skip = () => finish.current()
    window.addEventListener('keydown', skip, { capture: true, once: true })
    window.addEventListener('pointerdown', skip, { capture: true, once: true })
    window.addEventListener('kryo-pad-back', skip, { once: true })
    window.addEventListener('kryo-pad-confirm', skip, { once: true })
    // A hidden window gets no frames: never wait on them past this.
    const fallback = window.setTimeout(skip, LIGHTS_AT_MS + 1500 + HOLD_MS + FADE_MS)
    return () => {
      window.clearTimeout(sound)
      window.clearTimeout(fallback)
      window.removeEventListener('keydown', skip, { capture: true })
      window.removeEventListener('pointerdown', skip, { capture: true })
      window.removeEventListener('kryo-pad-back', skip)
      window.removeEventListener('kryo-pad-confirm', skip)
    }
  }, [])

  useEffect(() => {
    if (!lit) return
    const out = window.setTimeout(() => setLeaving(true), HOLD_MS)
    const end = window.setTimeout(() => finish.current(), HOLD_MS + FADE_MS)
    return () => {
      window.clearTimeout(out)
      window.clearTimeout(end)
    }
  }, [lit])

  return (
    <div
      aria-hidden
      className="pointer-events-auto fixed inset-0 z-[660] bg-black text-foreground transition-opacity ease-out"
      style={{ opacity: leaving ? 0 : 1, transitionDuration: `${FADE_MS}ms` }}
    >
      <SplashScene cell={cell} onRevealed={() => setLit(true)} />
      {lit ? (
        <p
          className="kryo-in pointer-events-none absolute inset-x-0 pl-[0.6em] text-center text-sm font-bold uppercase tracking-[0.6em] text-foreground"
          // Under the mark, which the scene centres 42% of the way down.
          style={{ top: `calc(42% + ${Math.round(cell * 44)}px)` }}
        >
          Big Picture
        </p>
      ) : null}
    </div>
  )
}
