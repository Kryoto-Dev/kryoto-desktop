import { FULL, HALF, LOW, TRACE } from './engine.ts'

/**
 * The 3x3 matrix's motions as numbers, for drawing many matrices on one
 * canvas: ui/Matrix.tsx and its keyframes ("The 3x3 matrix" in styles.css),
 * the same port the Discord art uses (kryo.to scripts/discord-assets/build.py,
 * `cell_ink`). Same states, same four inks, same hard-step timings. Cells are
 * 0 1 2 / 3 4 5 / 6 7 8.
 */

/** Perimeter, clockwise from top-left. */
export const RING = [0, 1, 2, 5, 8, 7, 6, 3] as const
/** Bottom row first, left to right: the order a container fills in. */
export const FILL_ORDER = [6, 7, 8, 3, 4, 5, 0, 1, 2] as const

export type WallState = 'busy' | 'sync' | 'scan' | 'connect' | 'process' | 'download'

/** The still frame of each state: ink per cell, the rest a trace. */
const FRAME: Record<WallState, Partial<Record<number, number>>> = {
  busy: { 0: FULL, 3: HALF, 6: LOW },
  sync: { 0: FULL, 3: HALF, 8: FULL, 5: HALF },
  scan: { 0: FULL, 3: FULL, 6: FULL, 1: LOW, 4: LOW, 7: LOW },
  connect: { 6: FULL, 4: FULL, 7: FULL },
  process: { 0: FULL, 2: FULL, 4: FULL, 6: FULL, 8: FULL },
  download: { 6: FULL, 7: FULL, 8: FULL },
}

/** One round of each motion, in ms (the CSS durations). */
export const LOOP_MS: Record<WallState, number> = { busy: 880, sync: 880, scan: 660, connect: 1320, process: 440, download: 1320 }

/** kryo-m-stage-n over 1320 ms: a trace until beat n of four, then lit. */
const stage = (t: number, n: number) => (t % 1320 >= n * 330 ? FULL : TRACE)

const mod = (a: number, n: number) => ((a % n) + n) % n

/** The ink of `cell` in a matrix showing `state`, `t` ms into it. */
export function cellInk(state: WallState, cell: number, t: number): number {
  const still = FRAME[state][cell] ?? TRACE
  switch (state) {
    case 'busy':
    case 'sync': {
      const k = (RING as readonly number[]).indexOf(cell)
      if (k < 0) return still
      let step = Math.floor(mod(t - k * 110, 880) / 110)
      if (state === 'sync') step %= 4
      return step === 0 ? FULL : step === 1 ? HALF : step === 2 ? LOW : TRACE
    }
    case 'download':
      return stage(t, 3 - Math.floor(cell / 3))
    case 'connect': {
      const col = cell % 3
      const row = Math.floor(cell / 3)
      return row < 2 - col ? still : stage(t, col + 1)
    }
    case 'scan': {
      const phase = mod(t - (cell % 3) * 220, 660)
      return phase < 220 ? FULL : phase < 440 ? LOW : TRACE
    }
    case 'process':
      return mod(t + (cell % 2 === 0 ? 0 : 220), 440) < 220 ? FULL : TRACE
  }
}
