import { dot, FULL, HALF, hash, LOW, OFF, STILL_MS, TRACE, type Scene, type VariantSpec } from './engine.ts'
import { MARK_COLS, MARK_DOTS, MARK_ROW_PITCH, MARK_ROWS } from './mark-data.ts'

/**
 * Mark: the Kryoto Discord server icon made big. The K// in matrix dots
 * (traced from kryo.to's own image, mark-data.ts) on a field of trace dots;
 * diagonal light sweeps pass through it in alternating directions, leaving a
 * wake that steps down the inks. Every minute it goes out dot by dot and
 * resolves in another corner; every other time the matrix's busy snake runs
 * round the place first, and the K// resolves after it, dot by dot.
 */

/** Dots a little smaller than the default: the mark and its board take about four tenths of the screen's width. */
const SCALE = 0.85
/** A new corner this often. */
const MOVE_MS = 60_000
/** Going out at the end of a minute, dot by dot. */
const LEAVE_MS = 1_600
/** Coming in, dot by dot. */
const RESOLVE_MS = 2_200
/** A sweep starts this often, and takes this long to cross. */
const SWEEP_EVERY_MS = 9_000
const SWEEP_MS = 4_200
/** How far past the mark a sweep lights the field, in dots. */
const SWEEP_MARGIN = 3
/** The slant of a sweep: the mark's diagonal strokes (build.py: 0.55 per character row). */
const SLANT = 0.55 * (2 / 4)
/** Every this many moves, the busy snake runs round before the mark resolves. */
const SNAKE_EVERY = 2
/** The snake moves one block this often, the matrix's own orbit step; one round. */
const SNAKE_STEP_MS = 110
/** Where the mark's middle sits, as shares of the screen: the four corners in turn. */
const CORNERS: readonly [number, number][] = [
  [0.33, 0.36],
  [0.67, 0.64],
  [0.67, 0.36],
  [0.33, 0.64],
]
/** The board of trace dots comes in, and goes out after the mark, over this long. */
const BOARD_MS = 900
/** One step down the inks takes this long when a dot goes out. */
const STEP_MS = 110

type Dot = { x: number; y: number; level: number; order: number; rank: number }

/** The mark's dots, with the order they resolve in: left to right, with grain so it is not a wipe. */
const DOTS: Dot[] = []
MARK_DOTS.forEach((line, y) =>
  Array.from(line).forEach((ch, x) => {
    if (ch === '.') return
    DOTS.push({ x, y, level: ch === '#' ? FULL : LOW, order: 0.7 * (x / MARK_COLS) + 0.3 * hash(x, y), rank: hash(y + 31, x) })
  }),
)

/** Evenly spaced starts from `a` to `b`, `n` of them, in whole dots. */
const spread = (a: number, b: number, n: number) => Array.from({ length: n }, (_, i) => Math.round(a + ((b - a) * i) / (n - 1)))

/**
 * The way round the mark for the busy snake: the board's margin cut into
 * 3 x 3 blocks of dots, clockwise from the top-left. Each block is one cell of
 * the matrix's orbit drawn large, so the snake is the matrix's own "working"
 * going round the mark.
 */
const RING: readonly [number, number][] = (() => {
  const m = SWEEP_MARGIN
  const xs = spread(-m, MARK_COLS, Math.round((MARK_COLS + 2 * m) / 3))
  const ys = spread(-m, MARK_ROWS, Math.round((MARK_ROWS + 2 * m) / 3))
  const out: [number, number][] = []
  xs.forEach((x) => out.push([x, ys[0]!]))
  ys.slice(1).forEach((y) => out.push([xs[xs.length - 1]!, y]))
  xs.slice(0, -1).reverse().forEach((x) => out.push([x, ys[ys.length - 1]!]))
  ys.slice(1, -1).reverse().forEach((y) => out.push([xs[0]!, y]))
  return out
})()

function make(scene: Scene) {
  const grid = scene.grid
  const { view, cols } = grid
  const jitter = Math.floor(scene.rand() * 4)

  const placeOf = (n: number) => {
    const [fx, fy] = CORNERS[(((n + jitter) % CORNERS.length) + CORNERS.length) % CORNERS.length]!
    const w = view.c1 - view.c0
    const h = view.r1 - view.r0
    const c0 = Math.round(view.c0 + fx * w - MARK_COLS / 2)
    const r0 = Math.round(view.r0 + fy * h - MARK_ROWS / 2)
    // Kept on screen, two dots in, whatever the screen's shape.
    return {
      c0: Math.max(view.c0 + 2, Math.min(view.c1 - MARK_COLS - 2, c0)),
      r0: Math.max(view.r0 + 2, Math.min(view.r1 - MARK_ROWS - 2, r0)),
    }
  }

  /**
   * The mark's board: trace dots under it and a little round it, as on the
   * server icon, the rest of the screen dark. It comes in dot by dot at a new
   * corner and goes out the same way.
   */
  const board = (c0: number, r0: number, local: number) => {
    const cells = grid.cells
    for (let y = -SWEEP_MARGIN; y < MARK_ROWS + SWEEP_MARGIN; y++) {
      const r = r0 + y
      if (r < 0 || r >= grid.rows) continue
      for (let x = -SWEEP_MARGIN; x < MARK_COLS + SWEEP_MARGIN; x++) {
        const c = c0 + x
        if (c < 0 || c >= cols) continue
        const rank = hash(x + 101, y + 7)
        if (local < rank * BOARD_MS) continue
        if (local >= MOVE_MS - BOARD_MS + rank * (BOARD_MS - STEP_MS)) continue
        cells[r * cols + c] = dot(TRACE)
      }
    }
  }

  return {
    frame(t: number) {
      const cells = grid.cells
      cells.fill(OFF)
      if (scene.reduced) {
        const { c0, r0 } = placeOf(Math.floor(t / STILL_MS))
        board(c0, r0, Infinity)
        for (const d of DOTS) cells[(r0 + d.y) * cols + c0 + d.x] = dot(d.level)
        return
      }

      const n = Math.floor(t / MOVE_MS)
      const local = t - n * MOVE_MS
      const { c0, r0 } = placeOf(n)
      board(c0, r0, local)
      const snake = n % SNAKE_EVERY === 1 && !scene.thumb
      const path = snake ? RING : null
      const arrive = path ? path.length * SNAKE_STEP_MS : 0
      const restFrom = arrive + RESOLVE_MS
      const leaveFrom = MOVE_MS - LEAVE_MS

      // The busy snake: a head and a two-step tail, as the matrix's orbit, each a block of dots.
      if (path && local < arrive + SNAKE_STEP_MS * 3) {
        const head = Math.floor(local / SNAKE_STEP_MS)
        for (let k = 2; k >= 0; k--) {
          const at = path[head - k]
          if (!at) continue
          const v = dot(k === 0 ? FULL : k === 1 ? HALF : LOW, k === 0)
          for (let y = 0; y < 3; y++) {
            for (let x = 0; x < 3; x++) {
              const c = c0 + at[0] + x
              const r = r0 + at[1] + y
              if (c >= 0 && r >= 0 && c < cols && r < grid.rows) cells[r * cols + c] = v
            }
          }
        }
      }

      // A sweep, if one is crossing now.
      const since = local - restFrom
      const sweep = since >= 0 && local < leaveFrom ? since % SWEEP_EVERY_MS : -1
      const crossing = sweep >= 0 && sweep < SWEEP_MS
      const reverse = Math.floor(since / SWEEP_EVERY_MS) % 2 === 1
      const span = MARK_COLS + MARK_ROWS * SLANT + 2 * SWEEP_MARGIN
      const head = crossing ? -SWEEP_MARGIN - 2 + (sweep / SWEEP_MS) * (span + 6) : 0
      const along = (x: number, y: number) => (reverse ? MARK_COLS - 1 - x : x) + y * SLANT

      if (crossing) {
        // The field around the mark catches the light too, and steps back down behind it.
        for (let y = -SWEEP_MARGIN; y < MARK_ROWS + SWEEP_MARGIN; y++) {
          for (let x = -SWEEP_MARGIN; x < MARK_COLS + SWEEP_MARGIN; x++) {
            const d = head - along(x, y)
            if (d < -1 || d >= 4) continue
            const c = c0 + x
            const r = r0 + y
            if (c < 0 || r < 0 || c >= cols || r >= grid.rows) continue
            const edge = x < 0 || y < 0 || x >= MARK_COLS || y >= MARK_ROWS
            cells[r * cols + c] = dot(d < 1 ? (edge ? LOW : HALF) : d < 2.5 && !edge ? LOW : TRACE)
          }
        }
      }

      for (const d of DOTS) {
        let level = TRACE
        const appear = arrive + d.order * RESOLVE_MS
        if (local >= appear) level = d.level
        if (local >= leaveFrom) {
          const out = local - (leaveFrom + d.rank * (LEAVE_MS - STEP_MS * 3))
          if (out >= 0) level = Math.max(TRACE, d.level - 1 - Math.floor(out / STEP_MS))
        } else if (crossing && local >= appear) {
          const dist = head - along(d.x, d.y)
          if (d.level === FULL) level = dist >= 1 && dist < 3.5 ? HALF : FULL
          else level = dist >= -1 && dist < 1 ? FULL : dist < 2.5 && dist >= 1 ? HALF : LOW
        }
        // A dot out (or not in yet) shows the board beneath, whatever it is doing.
        if (level !== TRACE) cells[(r0 + d.y) * cols + c0 + d.x] = dot(level)
      }
    },
  }
}

export const MARK_SAVER: VariantSpec = { id: 'mark', name: 'Mark', scale: SCALE, rowPitch: MARK_ROW_PITCH, make }
