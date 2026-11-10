import { useEffect, useMemo, useState } from 'react'
import { asciiPaths, gridSize } from '@/ui/ascii/cells'
import type { CellScene, Scene } from '@/ui/ascii/motion'
import { PixelMotion } from '@/ui/ascii/PixelMotion'
import { cn } from '@/lib/utils'

const FRAME_MS = 80

export type AnyScene = Scene | CellScene

function reducedMotion() {
  if (typeof window === 'undefined') return false
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches || document.documentElement.dataset.motion === 'reduced'
}

/**
 * Any of the moving pictures in `motion.ts`: pixel scenes through
 * `PixelMotion`, cell scenes as sharp vectors in the logo's lettering, with
 * their caption underneath. Size it by height (`className="h-12"`); the width
 * follows the picture. In the current text colour.
 */
export function SceneMotion({ scene, className, label }: { scene: AnyScene; className?: string; label?: string }) {
  if (!('kind' in scene)) return <PixelMotion scene={scene} className={className} label={label} />
  return <CellMotion scene={scene} className={className} label={label} />
}

/**
 * The most a scene ever needs: its largest frame and every caption it says.
 * Its box is that size from the first frame to the last, so the words beside
 * it never move and no card grows or shrinks as the story goes on.
 */
function extent(scene: CellScene) {
  let width = 0
  let height = 0
  const captions = new Set<string>()
  for (let i = 0; i < 64; i++) {
    const size = gridSize(scene.frame(i / 64))
    width = Math.max(width, size.width)
    height = Math.max(height, size.height)
    const caption = scene.caption?.(i / 64)
    if (caption) captions.add(caption)
  }
  return { width, height, captions: [...captions] }
}

function CellMotion({ scene, className, label }: { scene: CellScene; className?: string; label?: string }) {
  // With motion reduced it holds the end of the story: the second half, finished.
  const [t, setT] = useState(() => (reducedMotion() ? 0.99 : 0))
  useEffect(() => {
    if (reducedMotion()) return
    const start = performance.now()
    const id = window.setInterval(() => setT(((performance.now() - start) % scene.period) / scene.period), FRAME_MS)
    return () => window.clearInterval(id)
  }, [scene])
  const lines = scene.frame(t)
  const { width, height, captions } = useMemo(() => extent(scene), [scene])
  const caption = scene.caption?.(t)
  return (
    <span className="inline-grid max-w-full shrink-0 justify-items-start gap-1.5" role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <svg viewBox={`0 0 ${width} ${height}`} className={cn('block w-auto', className)} fill="currentColor">
        {asciiPaths(lines).map((p) => (
          <path key={p.alpha} d={p.d} fillOpacity={p.alpha < 1 ? p.alpha : undefined} />
        ))}
      </svg>
      {captions.length ? (
        <span className="grid text-[10px] font-bold uppercase tracking-wider text-muted-foreground">
          {captions.map((c) => (
            <span key={c} className={cn('col-start-1 row-start-1', c !== caption && 'invisible')}>
              {c}
            </span>
          ))}
        </span>
      ) : null}
    </span>
  )
}
