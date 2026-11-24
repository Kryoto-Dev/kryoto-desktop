import { dot, FULL, HALF, hash, LOW, OFF, STILL_MS, TRACE, type Scene, type VariantSpec } from './engine.ts'
import { MARK_COLS, MARK_DOTS, MARK_ROWS } from './mark-data.ts'
import { cellInk, LOOP_MS, type WallState } from './matrix-ink.ts'

/**
 * Snake: a wall of Kryoto's 3x3 matrices, every dot a trace at rest. Waves of
 * the matrix's own states travel across it, each matrix playing the state for
 * two rounds as the wave passes (orbit, sync, scan, signal, checker, fill),
 * and every few waves the lit dots of many matrices together spell the K//
 * (mark-data.ts, the same traced mark), hold, and let go.
 */

/** Dots per matrix side, and the gap after each, in dots. */
const SIDE = 3
const STRIDE = SIDE + 1
/** The waves, in turn. */
const WAVES: readonly WallState[] = ['busy', 'scan', 'sync', 'connect', 'process', 'download']
/** Matrices a second the front moves; how many rounds each matrix plays. */
const WAVE_SPEED = 2.4
const WAVE_ROUNDS = 2
/** The quiet between two waves. */
const WAVE_GAP_MS = 3_500
/** Every this many waves, the K//. */
const SPELL_EVERY = 3
/** The K// comes in dot by dot, holds, and goes out dot by dot. */
const SPELL_RESOLVE_MS = 2_600
const SPELL_HOLD_MS = 7_000
const SPELL_LEAVE_MS = 2_000
const STEP_MS = 110
/** The directions a wave may travel in: the diagonals, the K//'s own slant, so a wave never lights a whole column at once. */
const DIRECTIONS: readonly [number, number][] = [
  [0.8, 0.6],
  [-0.8, 0.6],
  [-0.8, -0.6],
  [0.8, -0.6],
]

type Episode = { kind: 'wave'; state: WallState; dir: [number, number]; ms: number } | { kind: 'spell'; ms: number; mc: number; mr: number }


function make(scene: Scene) {
  const grid = scene.grid
  const { view, cols, rows } = grid
  // Matrices that are wholly on screen.
  const mc0 = Math.ceil(view.c0 / STRIDE)
  const mr0 = Math.ceil(view.r0 / STRIDE)
  const mc1 = Math.floor((view.c1 + 1) / STRIDE)
  const mr1 = Math.floor((view.r1 + 1) / STRIDE)
  // The K// is drawn dot for dot on the wall's own grid, lighting the gaps it
  // crosses too, so the matrices join into one board for it and it keeps its shape.
  const markW = Math.ceil(MARK_COLS / STRIDE)
  const markH = Math.ceil(MARK_ROWS / STRIDE)
  const offset = Math.floor(scene.rand() * 1000)

  const episode = (k: number): Episode => {
    if (k % (SPELL_EVERY + 1) === SPELL_EVERY) {
      // A matrix of wall kept clear round it, so it never touches an edge.
      const mc = mc0 + 1 + Math.floor(hash(k, offset) * Math.max(1, mc1 - mc0 - markW - 1))
      const mr = mr0 + 1 + Math.floor(hash(offset, k) * Math.max(1, mr1 - mr0 - markH - 1))
      return { kind: 'spell', ms: SPELL_RESOLVE_MS + SPELL_HOLD_MS + SPELL_LEAVE_MS + WAVE_GAP_MS, mc, mr }
    }
    const state = WAVES[(k - Math.floor(k / (SPELL_EVERY + 1)) + offset) % WAVES.length]!
    const dir = DIRECTIONS[Math.floor(hash(k + offset, 5) * DIRECTIONS.length)]!
    const reach = Math.abs(dir[0]) * (mc1 - mc0) + Math.abs(dir[1]) * (mr1 - mr0)
    return { kind: 'wave', state, dir, ms: (reach / WAVE_SPEED) * 1000 + LOOP_MS[state] * WAVE_ROUNDS + WAVE_GAP_MS }
  }

  // Episodes run one after another; the current one and when it started.
  let k = 0
  let from = 1_200
  let ep = episode(0)

  const wall = (fill: number) => {
    const cells = grid.cells
    for (let r = 0; r < rows; r++) {
      const gapRow = r % STRIDE === SIDE
      for (let c = 0; c < cols; c++) cells[r * cols + c] = gapRow || c % STRIDE === SIDE ? OFF : fill
    }
  }

  const spell = (mc: number, mr: number, local: number, still: boolean) => {
    const leaveFrom = SPELL_RESOLVE_MS + SPELL_HOLD_MS
    MARK_DOTS.forEach((line, y) => {
      for (let x = 0; x < line.length; x++) {
        const ch = line[x]!
        if (ch === '.') continue
        const lit = ch === '#' ? FULL : LOW
        let level = lit
        const inGap = x % STRIDE === SIDE || y % STRIDE === SIDE
        if (!still) {
          const order = 0.7 * (x / MARK_COLS) + 0.3 * hash(x, y)
          if (local < order * SPELL_RESOLVE_MS) level = TRACE
          else if (local >= leaveFrom) {
            const out = local - leaveFrom - hash(y + 3, x) * (SPELL_LEAVE_MS - STEP_MS * 3)
            if (out >= 0) level = Math.max(TRACE, lit - 1 - Math.floor(out / STEP_MS))
          }
        }
        const c = mc * STRIDE + x
        const r = mr * STRIDE + y
        // A gap stays dark until the K// lights it.
        if (c < cols && r < rows) grid.cells[r * cols + c] = inGap && level === TRACE ? OFF : dot(level)
      }
    })
  }

  return {
    frame(t: number) {
      wall(dot(TRACE))
      if (scene.reduced) {
        const n = Math.floor(t / STILL_MS)
        const mc = mc0 + 1 + Math.floor(hash(n, 3) * Math.max(1, mc1 - mc0 - markW - 1))
        const mr = mr0 + 1 + Math.floor(hash(4, n) * Math.max(1, mr1 - mr0 - markH - 1))
        spell(mc, mr, 0, true)
        return
      }
      if (t < from) return
      while (t - from >= ep.ms) {
        from += ep.ms
        ep = episode(++k)
      }
      const local = t - from
      if (ep.kind === 'spell') return spell(ep.mc, ep.mr, local, false)

      // The wave: each matrix plays the state from when the front reaches it.
      const [dx, dy] = ep.dir
      const lo = Math.min(dx * mc0, dx * (mc1 - 1)) + Math.min(dy * mr0, dy * (mr1 - 1))
      const play = LOOP_MS[ep.state] * WAVE_ROUNDS
      for (let mr = mr0; mr < mr1; mr++) {
        for (let mc = mc0; mc < mc1; mc++) {
          const reached = ((dx * mc + dy * mr - lo) / WAVE_SPEED) * 1000
          const at = local - reached
          if (at < 0 || at >= play) continue
          // The last round fades a step: the wave's trailing edge is quieter.
          const tail = at >= play - LOOP_MS[ep.state] / 2
          for (let cell = 0; cell < 9; cell++) {
            let level = cellInk(ep.state, cell, at)
            if (tail && level > TRACE) level = level === FULL ? HALF : LOW
            const c = mc * STRIDE + (cell % 3)
            const r = mr * STRIDE + Math.floor(cell / 3)
            if (c < cols && r < rows) grid.cells[r * cols + c] = dot(level)
          }
        }
      }
    },
  }
}

// Rows as close as the mark's dots, so the K// spelled across the wall keeps its true shape.
export const SNAKE_SAVER: VariantSpec = { id: 'snake', name: 'Snake', make }
