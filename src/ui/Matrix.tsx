import type { CSSProperties } from 'react'
import { cn } from '@/lib/utils'

/**
 * Kryoto's 3x3 dot-matrix: a tiny teletext alphabet for system state.
 *
 * Every state is the same nine dots. What changes is which are lit and how
 * they move, and both follow a small grammar so that a glyph you have never
 * seen still reads as a sibling of the ones you have:
 *
 *   MOTION                         SHAPE
 *   orbit  something is working    check   it worked
 *   sync   two things agree        cross   it did not
 *   fill   something is arriving   caret   something newer exists
 *                                  plus    newly added
 *   scan   something is looking    slash   not available
 *   signal reaching a server       dot     idle, or a note
 *   blink  waiting on someone      grid    nothing here
 *   tick   in a queue
 *   checker  processing
 *
 * Motion is always hard steps (`step-end`), never an ease: a matrix switches,
 * it does not glide. Shapes "resolve" - their dots light one at a time in the
 * order you would draw the stroke - and then hold. With motion reduced, every
 * state is a still frame that still says the same thing.
 *
 * Four levels of ink, and nothing else: full, half, low, and a trace that
 * keeps the grid legible when a dot is "off".
 *
 * kryo.to's components/ui/matrix.tsx, copied: the Store in this window is the
 * site, and a state has to look the same on both sides of that edge. Keep the
 * two in step - the CSS is "The 3x3 matrix" in styles.css here and in the
 * site's globals.css. Pure markup and CSS, costing nothing while idle. Size with `size-*`, colour
 * with `text-*`: the dots are `currentColor`.
 */

const FULL = 1
const HALF = 0.55
const LOW = 0.3
export const TRACE = 0.14

/** Cells are numbered in reading order: 0 1 2 / 3 4 5 / 6 7 8. */
type Cell = 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8

/** Perimeter, clockwise from top-left. */
const RING: Cell[] = [0, 1, 2, 5, 8, 7, 6, 3]
/** Bottom row first, left to right: the order a container fills in. */
export const FILL_ORDER: Cell[] = [6, 7, 8, 3, 4, 5, 0, 1, 2]

type Motion = 'orbit' | 'sync' | 'fill' | 'scan' | 'signal' | 'blink' | 'tick' | 'checker' | 'resolve' | 'none'

type Spec = {
  motion: Motion
  /** The still frame: ink per cell (missing cells are a trace). */
  frame: Partial<Record<Cell, number>>
  /** For `resolve`: the stroke order the lit cells appear in. */
  stroke?: Cell[]
  /** Words for a screen reader when the glyph stands alone. */
  says: string
}

export const MATRIX_STATES = {
  // ── working ────────────────────────────────────────────────
  busy: { motion: 'orbit', frame: { 0: FULL, 3: HALF, 6: LOW }, says: 'Working' },
  sync: { motion: 'sync', frame: { 0: FULL, 3: HALF, 8: FULL, 5: HALF }, says: 'Syncing' },
  download: { motion: 'fill', frame: { 6: FULL, 7: FULL, 8: FULL }, says: 'Downloading' },
  scan: { motion: 'scan', frame: { 0: FULL, 3: FULL, 6: FULL, 1: LOW, 4: LOW, 7: LOW }, says: 'Checking' },
  connect: { motion: 'signal', frame: { 6: FULL, 4: FULL, 7: FULL }, says: 'Connecting' },
  wait: { motion: 'blink', frame: { 4: FULL }, says: 'Waiting' },
  queue: { motion: 'tick', frame: { 6: FULL, 7: FULL }, says: 'Queued' },
  process: { motion: 'checker', frame: { 0: FULL, 2: FULL, 4: FULL, 6: FULL, 8: FULL }, says: 'Processing' },
  // ── settled ────────────────────────────────────────────────
  success: { motion: 'resolve', frame: { 3: FULL, 7: FULL, 5: FULL, 2: FULL }, stroke: [3, 7, 5, 2], says: 'Done' },
  error: { motion: 'resolve', frame: { 0: FULL, 4: FULL, 8: FULL, 2: FULL, 6: FULL }, stroke: [0, 4, 8, 2, 6], says: 'Failed' },
  info: { motion: 'resolve', frame: { 1: HALF, 4: FULL, 7: FULL }, stroke: [1, 4, 7], says: 'Note' },
  update: { motion: 'resolve', frame: { 3: FULL, 1: FULL, 5: FULL, 7: LOW }, stroke: [3, 1, 5, 7], says: 'Update available' },
  added: { motion: 'resolve', frame: { 1: FULL, 3: FULL, 4: FULL, 5: FULL, 7: FULL }, stroke: [1, 4, 7, 3, 5], says: 'New' },
  complete: {
    motion: 'resolve',
    frame: { 0: FULL, 1: FULL, 2: FULL, 3: FULL, 4: FULL, 5: FULL, 6: FULL, 7: FULL, 8: FULL },
    stroke: FILL_ORDER,
    says: 'Complete',
  },
  online: { motion: 'none', frame: { 6: FULL, 4: FULL, 7: FULL, 2: FULL, 5: FULL, 8: FULL }, says: 'Online' },
  offline: { motion: 'none', frame: { 6: HALF }, says: 'Offline' },
  unavailable: { motion: 'none', frame: { 6: HALF, 4: HALF, 2: HALF }, says: 'Unavailable' },
  idle: { motion: 'none', frame: { 4: HALF }, says: 'Idle' },
  empty: { motion: 'none', frame: {}, says: 'Empty' },
} satisfies Record<string, Spec>

export type MatrixState = keyof typeof MATRIX_STATES

/** How each motion drives one cell. `null` keeps the cell still. */
function cellAnimation(motion: Motion, cell: Cell, spec: Spec): string | null {
  switch (motion) {
    case 'orbit':
    case 'sync': {
      const k = RING.indexOf(cell)
      if (k < 0) return null
      const name = motion === 'orbit' ? 'kryo-m-orbit' : 'kryo-m-orbit2'
      return `${name} 880ms step-end ${k * 110 - 880}ms infinite`
    }
    case 'fill': {
      // Rows arrive bottom-up: empty, one row, two, full, and round again.
      const stage = 3 - Math.floor(cell / 3)
      return `kryo-m-stage-${stage} 1320ms step-end infinite`
    }
    case 'signal': {
      // Three bars of rising height: columns 0, 1, 2 are one, two and three
      // dots tall. The cells above a bar stay a trace.
      const col = cell % 3
      const row = Math.floor(cell / 3)
      if (row < 2 - col) return null
      return `kryo-m-stage-${col + 1} 1320ms step-end infinite`
    }
    case 'tick': {
      if (cell < 6) return null
      return `kryo-m-stage-${cell - 5} 1320ms step-end infinite`
    }
    case 'scan': {
      const col = cell % 3
      return `kryo-m-scan 660ms step-end ${col * 220 - 660}ms infinite`
    }
    case 'blink':
      return cell === 4 ? 'kryo-m-blink 1400ms step-end infinite' : null
    case 'checker':
      return `kryo-m-blink 440ms step-end ${cell % 2 === 0 ? 0 : -220}ms infinite`
    case 'resolve': {
      const n = spec.stroke?.indexOf(cell) ?? -1
      if (n < 0) return null
      return `kryo-m-resolve 1ms step-end ${90 + n * 60}ms both`
    }
    default:
      return null
  }
}

/**
 * Whole-pixel dots. With 1fr tracks and a percentage gap, a small matrix
 * (12-16 px) puts its dots on fractions of a pixel and the browser draws some
 * wider than others: stretched cubes. For a Tailwind size it knows, the
 * matrix lays itself out in whole pixels instead - every dot the same size,
 * whole-pixel gaps, the leftover pixel at the edges. Dots under 6 px lose
 * their rounding, which at that size only blurs their corners. At a screen
 * scale like 125% the page re-fits it in device pixels (pixel-icons-motion).
 */
const SIZE_STEP = /(?:^|\s)size-(\d+(?:\.5)?)(?=\s|$)/
function wholePixels(classes: string): CSSProperties | undefined {
  const step = classes.match(SIZE_STEP)?.[1]
  if (!step) return undefined
  const box = parseFloat(step) * 4
  const dot = Math.max(2, Math.floor(box * 0.29))
  const gap = Math.max(1, Math.floor((box - 3 * dot) / 2.6))
  const edge = Math.max(0, Math.floor((box - 3 * dot - 2 * gap) / 2))
  return {
    gridTemplateColumns: `repeat(3, ${dot}px)`,
    gridTemplateRows: `repeat(3, ${dot}px)`,
    gap: `${gap}px`,
    placeContent: 'start',
    padding: `${edge}px 0 0 ${edge}px`,
    boxSizing: 'border-box',
    ...(dot < 6 ? { '--kryo-m-r': '0' } : null),
  } as CSSProperties
}

/**
 * One glyph of the alphabet.
 *
 * `state` picks the glyph. `progress` (0-1) instead draws a determinate fill:
 * cells light in fill order, bottom row first, and the next one to fill
 * blinks - a nine-step meter that is still visibly alive.
 *
 * Decorative unless given `label` (or `announce`, which reads the state's own
 * words); a glyph beside text that already says it should stay silent.
 */
export function Matrix({
  state = 'busy',
  progress,
  label,
  announce = false,
  className,
}: {
  state?: MatrixState
  progress?: number | null
  label?: string
  announce?: boolean
  className?: string
}) {
  const spec: Spec = MATRIX_STATES[state]
  const determinate = typeof progress === 'number' && Number.isFinite(progress)
  const lit = determinate ? Math.floor(Math.max(0, Math.min(1, progress)) * 9) : 0
  const head = determinate && lit < 9 ? FILL_ORDER[lit] : null

  const words = label ?? (announce ? spec.says : undefined)
  const a11y = words
    ? { role: determinate || spec.motion !== 'resolve' ? 'status' : 'img', 'aria-label': words }
    : { 'aria-hidden': true as const }

  return (
    <span
      className={cn('kryo-matrix size-3.5 shrink-0', className)}
      style={wholePixels(cn('kryo-matrix size-3.5 shrink-0', className))}
      data-state={determinate ? 'progress' : state}
      {...a11y}
    >
      {Array.from({ length: 9 }, (_, i) => {
        const cell = i as Cell
        let ink: number
        let animation: string | null
        if (determinate) {
          const order = FILL_ORDER.indexOf(cell)
          ink = order < lit ? FULL : TRACE
          animation = cell === head ? 'kryo-m-blink 880ms step-end infinite' : null
          if (cell === head) ink = HALF
        } else {
          ink = spec.frame[cell] ?? TRACE
          animation = cellAnimation(spec.motion, cell, spec)
        }
        const style = { '--o': ink, ...(animation ? { animation } : null) } as CSSProperties
        return <i key={i} style={style} />
      })}
    </span>
  )
}
