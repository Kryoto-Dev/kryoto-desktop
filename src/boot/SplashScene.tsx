import { useLayoutEffect, useRef } from 'react'
import { asciiPaths, CELL_H, CELL_W, SCRAMBLE, word } from '@/ui/ascii/cells'
import { cn } from '@/lib/utils'

/**
 * The splash's opening, drawn entirely in Kryoto's block-and-double-line
 * lettering (the cells `cells.ts` draws, so every character is crisp).
 *
 * Little ASCII game cases spiral in from the edges, shrinking as they fall,
 * each with a comet tail of shade characters, and are drawn into a ghost of
 * the K//. Every one that lands fills the mark in a little further; the
 * last one lights it: the mark scrambles and settles solid in the site's
 * voice, and the site's shimmer runs across it.
 *
 * One canvas, redrawn every frame from the clock, so it is as smooth as the
 * display and the same length on a slow machine as a fast one.
 */

/** One character cell on screen, in CSS pixels (the lettering's 3:5), at `cell` 1. */
const BASE_CW = 6
const BASE_CH = 10
/** The start box (lib/window.ts, `splash`): the scene is drawn for this at `cell` 1. */
const BOX_W = 340
const BOX_H = 380

const MARK = word('K//')

/** The cases that fly in: big while far out, then smaller as they fall. */
const CASE_FILLS = ['▓▓▓▓', '▒▒▒▒', '▀▄▀▄', '░░░░', '████', '▄▀▄▀', '▒▓▒▓']
const caseBig = (fill: string) => ['╔════╗', `║${fill}║`, `║${fill}║`, `║${fill}║`, '╚════╝']
const caseSmall = (fill: string) => ['╔══╗', `║${fill.slice(0, 2)}║`, '╚══╝']

const CARDS = 10
const FIRST_MS = 60
const STAGGER_MS = 85
const FLIGHT_MS = 920
const BURST_MS = 1100
const REVEAL_AFTER_BURST_MS = 420

/** Slow start, then pulled in hard. */
const pull = (p: number) => Math.pow(p, 2.3)
const clamp = (v: number, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, v))

/** A stable pseudo-random number in [0, 1). */
function rand(a: number, b = 0) {
  const n = Math.sin(a * 127.1 + b * 311.7 + 7.3) * 43758.5453
  return n - Math.floor(n)
}

function reducedMotion() {
  return typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
}

/**
 * The characters scale with the room there is, 1 in the start box, so the mark
 * holds the same share of any window, and stays crisp, not stretched. `fit`
 * adjusts that share. It is laid out again when the size changes: the scene
 * can start before its window has settled (Big Picture going full screen, a
 * window a compositor sized itself). `onMark` says where the mark ends, for
 * whatever goes under it.
 * `settled` draws the mark as the opening leaves it, at once (Kryoto Mobile's
 * Welcome after a sign-out).
 */
export function SplashScene({
  onRevealed,
  className,
  fit = 1,
  onMark,
  settled = false,
}: {
  onRevealed: () => void
  className?: string
  fit?: number
  /** The mark's bottom edge in the scene, and the scale it is drawn at. */
  onMark?: (bottom: number, cell: number) => void
  settled?: boolean
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)

  useLayoutEffect(() => {
    const canvas = canvasRef.current
    const ctx = canvas?.getContext('2d')
    if (!canvas || !ctx) {
      onRevealed()
      return
    }

    let told = false
    const reveal = () => {
      if (told) return
      told = true
      onRevealed()
    }

    // ---- The grid -----------------------------------------------------------
    const color = getComputedStyle(canvas).color
    const markCols = Math.max(...MARK.map((l) => Array.from(l).length))
    const markRows = MARK.length
    let width = 1
    let height = 1
    let dpr = 1
    let CW = BASE_CW
    let CH = BASE_CH
    let cols = 0
    let rows = 0
    let ox = 0
    let oy = 0
    let cx = 0
    let cy = 0
    let mc0 = 0
    let mr0 = 0
    let reach = 0
    let markCells: { c: number; r: number; ch: string; order: number }[] = []
    let dust: { c: number; r: number; speed: number; phase: number }[] = []
    let heat = new Float32Array(0)

    const measure = () => {
      const box = canvas.getBoundingClientRect()
      width = Math.max(1, box.width)
      height = Math.max(1, box.height)
      dpr = window.devicePixelRatio || 1
      const cell = Math.max(0.5, Math.min(width / BOX_W, height / BOX_H) * fit)
      CW = BASE_CW * cell
      CH = BASE_CH * cell
      canvas.width = Math.round(width * dpr)
      canvas.height = Math.round(height * dpr)
      cols = Math.floor(width / CW)
      rows = Math.floor(height / CH)
      ox = (width - cols * CW) / 2
      oy = (height - rows * CH) / 2

      // The mark, a little above the middle, in cells and in pixels.
      mc0 = Math.round((cols - markCols) / 2)
      mr0 = Math.round(rows * 0.42 - markRows / 2)
      cx = ox + (mc0 + markCols / 2) * CW
      cy = oy + (mr0 + markRows / 2) * CH
      markCells = []
      MARK.forEach((line, r) =>
        Array.from(line).forEach((ch, c) => {
          if (ch === ' ') return
          // Fills left to right, with some grain so it does not read as a wipe.
          markCells.push({ c: mc0 + c, r: mr0 + r, ch, order: 0.62 * (c / markCols) + 0.38 * rand(c, r) })
        }),
      )

      // Faint dust in the dark, twinkling.
      dust = Array.from({ length: Math.round((cols * rows) / 40) }, (_, i) => ({
        c: Math.floor(rand(i, 1) * cols),
        r: Math.floor(rand(i, 2) * rows),
        speed: 0.8 + rand(i, 3) * 1.6,
        phase: rand(i, 4) * Math.PI * 2,
      }))
      reach = Math.hypot(width, height) * 0.6
      heat = new Float32Array(cols * rows)
      onMark?.(oy + (mr0 + markRows) * CH, cell)
    }
    measure()

    // ---- Who flies where ----------------------------------------------------
    const cards = Array.from({ length: CARDS }, (_, i) => ({
      start: FIRST_MS + i * STAGGER_MS,
      theta0: (i / CARDS) * Math.PI * 2 - Math.PI / 2 + (rand(i, 5) - 0.5) * 0.6,
      extra: rand(i, 6) * 30,
      swirl: (i % 2 ? 1 : -1) * (0.8 + rand(i, 7) * 0.7),
      fill: CASE_FILLS[i % CASE_FILLS.length]!,
      done: false,
    }))
    const burstAt = FIRST_MS + (CARDS - 1) * STAGGER_MS + FLIGHT_MS

    let landed = 0
    let lastLanding = -Infinity
    let burst = false

    // Settled: the mark as the opening leaves it, at once.
    const still = settled || reducedMotion()
    const t0 = performance.now()
    let last = t0
    let raf = 0
    const timers: number[] = []

    // ---- One frame ----------------------------------------------------------
    const frame = (now: number) => {
      const t = still ? burstAt + BURST_MS + 2000 : now - t0
      const dt = Math.min(0.05, Math.max(0, now - last) / 1000)
      last = now

      // Two layers: what glows faintly behind, and what is solid.
      const dim: string[][] = Array.from({ length: rows }, () => Array<string>(cols).fill(' '))
      const solid: string[][] = Array.from({ length: rows }, () => Array<string>(cols).fill(' '))
      const put = (layer: string[][], c: number, r: number, ch: string) => {
        if (r >= 0 && r < rows && c >= 0 && c < cols) layer[r]![c] = ch
      }

      // Dust.
      if (!still) {
        const fade = clamp(t / 700)
        for (const d of dust) {
          const s = Math.sin((t / 1000) * d.speed * 2 + d.phase)
          if (s * fade > 0.55) put(dim, d.c, d.r, '·')
        }
      }

      // Tails cool down.
      const cool = Math.exp(-dt * 4.5)
      for (let i = 0; i < heat.length; i++) heat[i] = heat[i]! * cool

      // Cases in flight.
      if (!still) {
        for (const card of cards) {
          const p = (t - card.start) / FLIGHT_MS
          if (p < 0) continue
          if (p >= 1) {
            if (!card.done) {
              card.done = true
              landed++
              lastLanding = t
            }
            continue
          }
          const e = pull(p)
          const theta = card.theta0 + card.swirl * e
          const r = (reach + card.extra) * (1 - e)
          const x = cx + Math.cos(theta) * r
          const y = cy + Math.sin(theta) * r * 0.85
          const sprite = e < 0.5 ? caseBig(card.fill) : e < 0.78 ? caseSmall(card.fill) : e < 0.92 ? ['▪'] : ['·']
          const sw = Array.from(sprite[0]!).length
          const c0 = Math.round((x - ox) / CW - sw / 2)
          const r0 = Math.round((y - oy) / CH - sprite.length / 2)
          sprite.forEach((line, dr) => Array.from(line).forEach((ch, dc) => put(solid, c0 + dc, r0 + dr, ch)))
          // The tail starts from the middle of the case: one dotted line.
          const tc = Math.round((x - ox) / CW - 0.5)
          const tr = Math.round((y - oy) / CH - 0.5)
          if (tc >= 0 && tc < cols && tr >= 0 && tr < rows) heat[tr * cols + tc] = 1
        }
      }

      // The burst.
      if (!burst && t >= burstAt) {
        burst = true
        landed = CARDS
        timers.push(window.setTimeout(reveal, still ? 0 : REVEAL_AFTER_BURST_MS))
      }

      // Tails, under everything solid: dots that go out.
      for (let r = 0; r < rows; r++) {
        for (let c = 0; c < cols; c++) {
          const h = heat[r * cols + c]!
          if (h > 0.1 && solid[r]![c] === ' ') put(dim, c, r, h > 0.45 ? '▪' : '·')
        }
      }

      // The mark: a ghost that each landed case fills further in.
      const sinceBurst = t - burstAt
      const charge = 0.06 + 0.82 * (landed / CARDS)
      // Each landing brightens the mark for a moment.
      const pulse = clamp(1 - (t - lastLanding) / 260)
      const tick = Math.floor(t / 34)
      for (const m of markCells) {
        let ch: string
        if (burst && sinceBurst < 460 && !still) {
          // Scrambles and settles, the site's voice.
          ch = rand(m.c, m.r + tick) < 1 - sinceBurst / 460 ? SCRAMBLE[Math.floor(rand(m.r, m.c + tick) * SCRAMBLE.length)]! : m.ch
        } else if (burst) {
          ch = m.ch
          // The shimmer: a diagonal band of lighter blocks, once every 1.6s.
          if (ch === '█' && !still) {
            const span = markCols + markRows + 10
            const pos = (((sinceBurst - 460) % 1600) / 1600) * span - 5
            const d = Math.abs(m.c - mc0 + (m.r - mr0) * 0.8 - pos)
            if (d < 1) ch = '▒'
            else if (d < 2.2) ch = '▓'
          }
        } else {
          const v = charge + pulse * 0.18 - m.order
          ch = v > 0.22 ? m.ch : v > 0.08 ? '▓' : v > -0.06 ? '▒' : '░'
        }
        put(solid, m.c, m.r, ch)
      }

      // ---- Paint ------------------------------------------------------------
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
      ctx.clearRect(0, 0, width, height)
      ctx.translate(ox, oy)
      ctx.scale(CW / CELL_W, CH / CELL_H)
      ctx.fillStyle = color
      const intro = still ? 1 : clamp(t / 450)
      for (const [layer, alpha] of [[dim, 0.55], [solid, 1]] as const) {
        for (const p of asciiPaths(layer.map((row) => row.join('')))) {
          ctx.globalAlpha = alpha * p.alpha * intro
          ctx.fill(new Path2D(p.d))
        }
      }
      ctx.globalAlpha = 1

      if (!still) raf = requestAnimationFrame(frame)
    }

    if (still) frame(performance.now())
    else raf = requestAnimationFrame(frame)

    const follow = () => {
      const box = canvas.getBoundingClientRect()
      if (box.width === width && box.height === height && (window.devicePixelRatio || 1) === dpr) return
      measure()
      if (still) frame(performance.now())
    }
    const watch = new ResizeObserver(follow)
    watch.observe(canvas)
    // A window moved to a screen with another scale changes no size.
    window.addEventListener('resize', follow)

    // Whatever happens to the frames (a hidden window gets none), the splash
    // never waits on them past this.
    timers.push(window.setTimeout(reveal, burstAt + 1500))

    return () => {
      cancelAnimationFrame(raf)
      watch.disconnect()
      window.removeEventListener('resize', follow)
      for (const id of timers) window.clearTimeout(id)
    }
    // Runs once: the scene is a one-off.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <canvas
      ref={canvasRef}
      role="img"
      aria-label="Kryoto"
      className={cn('pointer-events-none absolute inset-0 size-full text-foreground', className)}
    />
  )
}
