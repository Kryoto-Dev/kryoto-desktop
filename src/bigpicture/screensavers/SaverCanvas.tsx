import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { SplashScene } from '@/boot/SplashScene'
import { reducedMotion } from '@/lib/motion'
import { cn } from '@/lib/utils'
import { mountSaver, STILL_MS } from './engine'
import { saverSpec, type SaverId } from './index'

/** In the page server only: frames drawn and their cost, read from the console as `__kryoSaverFrames`. */
function countFrame(ms: number) {
  const w = window as unknown as { __kryoSaverFrames?: { n: number; total: number; longest: number } }
  const s = (w.__kryoSaverFrames ??= { n: 0, total: 0, longest: 0 })
  s.n++
  s.total += ms
  s.longest = Math.max(s.longest, ms)
}

/**
 * Field's start-screen scene starts over somewhere else this often, in whole
 * pixels. With motion reduced, only as often as the cells' still changes.
 */
const SPLASH_MOVE_MS = 90_000
/**
 * Where it starts over: the middle, then four places a modest step from it,
 * in turn (vw, vh). Field's names sit in the bands above and below, clear of
 * the mark wherever it is.
 */
const SPLASH_PLACES: readonly [number, number][] = [
  [0, 0],
  [-5, -4],
  [5, 4],
  [5, -4],
  [-5, 4],
]
/**
 * As a screensaver, the start screen's scene draws half as often as the
 * cells around it: it redraws its whole picture each frame, and its shimmer
 * reads the same at 10. The start screen itself is unchanged.
 */
const SPLASH_FPS_SHARE = 0.5

/**
 * One screensaver drawn into the box it is put in: full screen in Big Picture,
 * or small and slow as a thumbnail in Settings. The cell engine draws it on
 * one canvas; Field also asks for the start screen's own scene underneath.
 * Titles are read when needed, so a library that changes never restarts it.
 */
export function SaverCanvas({
  id,
  fps,
  titles,
  lastPlayed,
  thumb = false,
  className,
}: {
  id: SaverId
  fps: number
  titles: readonly string[]
  lastPlayed: string | null
  thumb?: boolean
  className?: string
}) {
  const canvas = useRef<HTMLCanvasElement | null>(null)
  const data = useRef({ titles, lastPlayed })
  data.current = { titles, lastPlayed }
  const [underlay, setUnderlay] = useState<'splash' | null>(null)

  useLayoutEffect(() => {
    const el = canvas.current
    if (!el) return
    return mountSaver(el, saverSpec(id), {
      fps,
      thumb,
      titles: () => data.current.titles,
      lastPlayed: () => data.current.lastPlayed,
      onUnderlay: setUnderlay,
      // The page server only: what each frame cost, for checking it stays cheap.
      onFrame: import.meta.env.DEV && !thumb ? countFrame : undefined,
    })
  }, [id, fps, thumb])

  return (
    <div className={cn('absolute inset-0 overflow-hidden', className)}>
      {underlay === 'splash' ? <Splash fps={Math.max(1, Math.round(fps * SPLASH_FPS_SHARE))} moves={!thumb} /> : null}
      <canvas ref={canvas} aria-hidden className="pointer-events-none text-foreground" />
    </div>
  )
}

/** The start screen's scene, dimmed, starting over somewhere new now and then. */
function Splash({ fps, moves }: { fps: number; moves: boolean }) {
  const [turn, setTurn] = useState(0)
  useEffect(() => {
    if (!moves) return
    const t = window.setInterval(() => setTurn((n) => n + 1), reducedMotion() ? STILL_MS : SPLASH_MOVE_MS)
    return () => window.clearInterval(t)
  }, [moves])
  const [x, y] = SPLASH_PLACES[turn % SPLASH_PLACES.length]!
  return (
    <div className="absolute inset-0 opacity-45" style={{ transform: `translate(${Math.round((x * window.innerWidth) / 100)}px, ${Math.round((y * window.innerHeight) / 100)}px)` }}>
      <SplashScene key={turn} fps={fps} fit={0.7} onRevealed={() => {}} />
    </div>
  )
}
