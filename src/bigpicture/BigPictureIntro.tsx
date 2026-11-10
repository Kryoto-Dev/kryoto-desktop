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

/** How long BIG PICTURE holds once the mark is lit, then the fade. */
const HOLD_MS = 650
const FADE_MS = 350
/** The scene lights the mark this long after it starts (SplashScene's burst). */
const LIGHTS_AT_MS = 1745
/** Where the sound's chord sits (sound-recipes.ts, `bpIntro`). */
const CHORD_AT_MS = 900

function reducedMotion() {
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
  // Where BIG PICTURE goes, from where the scene says the mark ends: it follows
  // the window, which is still growing as Big Picture opens.
  const [label, setLabel] = useState(0)

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
      className="kryo-night pointer-events-auto fixed inset-0 z-[660] bg-black text-foreground transition-opacity ease-out"
      style={{ opacity: leaving ? 0 : 1, transitionDuration: `${FADE_MS}ms` }}
    >
      <SplashScene fit={0.85} onMark={(bottom, cell) => setLabel(bottom + cell * 14)} onRevealed={() => setLit(true)} />
      {lit ? (
        <p
          className="kryo-in pointer-events-none absolute inset-x-0 pl-[0.6em] text-center text-sm font-bold uppercase tracking-[0.6em] text-foreground"
          style={{ top: label }}
        >
          Big Picture
        </p>
      ) : null}
    </div>
  )
}
