import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { SplashScene } from '@/boot/SplashScene'
import { cn } from '@/lib/utils'
import { mountSaver } from './engine'
import { saverSpec, type SaverId } from './index'

/** In the page server only: frames drawn and their cost, read from the console as `__kryoSaverFrames`. */
function countFrame(ms: number) {
  const w = window as unknown as { __kryoSaverFrames?: { n: number; total: number; longest: number } }
  const s = (w.__kryoSaverFrames ??= { n: 0, total: 0, longest: 0 })
  s.n++
  s.total += ms
  s.longest = Math.max(s.longest, ms)
}

/** Field's start-screen scene starts over somewhere else this often (as the first screensaver did). */
const SPLASH_MOVE_MS = 40_000
/** Somewhere new on the screen, within a tenth of it from the middle. */
const place = () => ({ x: Math.round((Math.random() - 0.5) * 20), y: Math.round((Math.random() - 0.5) * 16) })

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
      drift: !thumb,
      titles: () => data.current.titles,
      lastPlayed: () => data.current.lastPlayed,
      onUnderlay: setUnderlay,
      // The page server only: what each frame cost, for checking it stays cheap.
      onFrame: import.meta.env.DEV && !thumb ? countFrame : undefined,
    })
  }, [id, fps, thumb])

  return (
    <div className={cn('absolute inset-0 overflow-hidden', className)}>
      {underlay === 'splash' ? <Splash fps={fps} moves={!thumb} /> : null}
      <canvas ref={canvas} aria-hidden className="pointer-events-none text-foreground" />
    </div>
  )
}

/** The start screen's scene, dimmed, starting over somewhere new now and then. */
function Splash({ fps, moves }: { fps: number; moves: boolean }) {
  const [turn, setTurn] = useState(0)
  const [at, setAt] = useState({ x: 0, y: 0 })
  useEffect(() => {
    if (!moves) return
    const t = window.setInterval(() => {
      setTurn((n) => n + 1)
      setAt(place())
    }, SPLASH_MOVE_MS)
    return () => window.clearInterval(t)
  }, [moves])
  return (
    <div className="absolute inset-0 opacity-45" style={{ transform: `translate(${at.x}vw, ${at.y}vh)` }}>
      <SplashScene key={turn} fps={fps} fit={0.7} onRevealed={() => {}} />
    </div>
  )
}
