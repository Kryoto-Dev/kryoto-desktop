import { useEffect, useRef } from 'react'
import { Canvas, SCENE_H, SCENE_W, type Scene } from '@/ui/ascii/motion'
import { cn } from '@/lib/utils'

const FRAME_MS = 80
const DIM_ALPHA = 0.42

function reducedMotion() {
  if (typeof window === 'undefined') return false
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches || document.documentElement.dataset.motion === 'reduced'
}

/**
 * A `Scene` (ascii/motion.ts) playing on a loop, drawn on a canvas with every
 * pixel a whole number of device pixels: square, all the same size, crisp at
 * any screen scale. Size it by height (`className="h-12"`): the picture takes
 * the largest whole-pixel scale that fits, and its width follows. In the
 * current text colour. With motion reduced it holds the finished picture.
 */
export function PixelMotion({ scene, className, label }: { scene: Scene; className?: string; label?: string }) {
  const box = useRef<HTMLSpanElement>(null)
  const canvas = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const host = box.current
    const el = canvas.current
    const ctx = el?.getContext('2d')
    if (!host || !el || !ctx) return
    let scale = 1
    const fit = () => {
      const dpr = window.devicePixelRatio || 1
      const tall = host.clientHeight || SCENE_H * 2
      scale = Math.max(1, Math.floor((tall * dpr) / SCENE_H))
      el.width = SCENE_W * scale
      el.height = SCENE_H * scale
      el.style.width = `${(SCENE_W * scale) / dpr}px`
      el.style.height = `${(SCENE_H * scale) / dpr}px`
    }
    const paint = (t: number) => {
      const frame = new Canvas()
      scene.draw(frame, t)
      ctx.clearRect(0, 0, el.width, el.height)
      ctx.fillStyle = getComputedStyle(host).color
      for (const [tone, alpha] of [[2, 1], [1, DIM_ALPHA]] as const) {
        ctx.globalAlpha = alpha
        for (let y = 0; y < SCENE_H; y++)
          for (let x = 0; x < SCENE_W; x++) if (frame.px[y * SCENE_W + x] === tone) ctx.fillRect(x * scale, y * scale, scale, scale)
      }
      ctx.globalAlpha = 1
    }
    fit()
    const still = reducedMotion()
    const start = performance.now()
    const tick = () => paint(still ? 0.99 : ((performance.now() - start) % scene.period) / scene.period)
    tick()
    // The canvas sets the host's width, so refitting from inside the observer's
    // callback resized what it watches ("ResizeObserver loop", KB-0251): only a
    // new height refits, on the next frame.
    let tall = host.clientHeight
    let frame = 0
    const observer = new ResizeObserver(() => {
      if (host.clientHeight === tall) return
      tall = host.clientHeight
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        fit()
        tick()
      })
    })
    observer.observe(host)
    const id = still ? null : window.setInterval(tick, FRAME_MS)
    return () => {
      cancelAnimationFrame(frame)
      observer.disconnect()
      if (id) window.clearInterval(id)
    }
  }, [scene])

  return (
    <span ref={box} className={cn('inline-flex shrink-0 items-center', className)} role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <canvas ref={canvas} className="block" />
    </span>
  )
}
