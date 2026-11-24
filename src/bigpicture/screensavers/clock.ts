import { dot, FULL, glyph, HALF, hash, LOW, OFF, restSpots, STILL_MS, TRACE, type Scene, type VariantSpec } from './engine.ts'
import { clockDigits, COLON, NUMERAL_H, NUMERAL_W, numeralDot, systemTwelveHour } from './numerals.ts'
import { saverText } from './titles.ts'

/**
 * Clock: the time in matrix-dot numerals, big enough to read from a couch,
 * on a board of trace dots; the date and KRYOTO, then the game played last,
 * small beneath as a quieter block. A numeral changes by its dots switching,
 * one by one, never by sliding; the colon breathes slowly in the reader's
 * accent, in soft steps. Every minute and a half the whole clock goes out dot
 * by dot and comes back at the next of five balanced places, the numerals
 * always the visual centre. 12 or 24 hours as the system shows them.
 */

/** Cells a little larger than the default screensavers'. */
const SCALE = 1.15
/** Each dot of a numeral is this many cells square: the numerals read from across a room. */
const DOT = 2
/** A new place this often. */
const MOVE_MS = 90_000
/** Going out and coming in, dot by dot. */
const LEAVE_MS = 1_400
const RESOLVE_MS = 1_800
/** A numeral's dots switch over this long when the minute turns. */
const SWITCH_MS = 700
/** One dot's switch: a beat at half ink between off and on. */
const STEP_MS = 90
/** The colon breathes: lit, a step down, low, a step up (ms each). Slow and soft, never a flash. */
const BREATH: readonly [number, number][] = [
  [1_200, FULL],
  [300, HALF],
  [1_200, LOW],
  [300, HALF],
]
const BREATH_MS = BREATH.reduce((a, [ms]) => a + ms, 0)
/** Gaps in numeral dots: between numerals, and either side of the colon. */
const GAP = 1
/** Cells between the numerals and the lines of text below them. */
const TEXT_GAP = 3
/** The board of trace dots reaches this far past the clock, in cells. */
const PANEL_SIDE = 3
const PANEL_EDGE = 2

/** The numerals' columns, in numeral dots, left to right: four numerals and the colon. */
function slots() {
  const out: { x: number; kind: 'digit' | 'colon'; i: number }[] = []
  let x = 0
  for (let i = 0; i < 4; i++) {
    out.push({ x, kind: 'digit', i })
    x += NUMERAL_W + GAP
    if (i === 1) {
      out.push({ x, kind: 'colon', i: -1 })
      x += 1 + GAP
    }
  }
  return out
}
const SLOTS = slots()
/** The numerals' width in cells (four numerals and the colon). */
const NUMERALS_W = (4 * NUMERAL_W + 3 * GAP + 1 + GAP) * DOT
const NUMERALS_H = NUMERAL_H * DOT
/** AM or PM sits after the numerals, two cells clear, and does not move them off centre. */
const AMPM_W = 4

function breath(t: number): number {
  let at = ((t % BREATH_MS) + BREATH_MS) % BREATH_MS
  for (const [ms, level] of BREATH) {
    if (at < ms) return level
    at -= ms
  }
  return FULL
}

function dateLine(now: Date): string {
  const day = new Intl.DateTimeFormat(undefined, { weekday: 'short', day: 'numeric', month: 'short' }).format(now)
  return `${saverText(day, 18)} // KRYOTO`
}

function make(scene: Scene) {
  const grid = scene.grid
  const { view, cols, rows } = grid
  const twelve = systemTwelveHour()
  // The board: the numerals, with room either side for AM/PM so the numerals stay centred.
  const side = PANEL_SIDE + (twelve ? AMPM_W : 0)
  const boardW = NUMERALS_W + 2 * side
  const boardH = NUMERALS_H + TEXT_GAP + 3 + 2 * PANEL_EDGE
  // The numerals are the visual centre: the places are worked out for them,
  // and the quieter lines of text hang below (kept on screen).
  const numeralsBox = NUMERALS_H + 2 * PANEL_EDGE
  const spots = restSpots(view, boardW, numeralsBox).map((p) => ({ c0: p.c0, r0: Math.max(view.r0, Math.min(p.r0, view.r1 - boardH)) }))
  const first = Math.floor(scene.rand() * spots.length)
  const placeOf = (n: number) => {
    const spot = scene.thumb ? spots[0]! : spots[(n + first) % spots.length]!
    return { c0: spot.c0 + side, r0: spot.r0 + PANEL_EDGE, board: spot }
  }

  let shownDigits = ''
  let prevDigits = ''
  let changedAt = -Infinity

  const set = (c: number, r: number, v: number) => {
    if (c >= 0 && r >= 0 && c < cols && r < rows) grid.cells[r * cols + c] = v
  }

  return {
    frame(t: number) {
      grid.cells.fill(OFF)
      const now = new Date()
      const { digits, pm } = clockDigits(now, twelve)
      if (digits !== shownDigits) {
        prevDigits = shownDigits || digits
        shownDigits = digits
        changedAt = t
      }
      const still = scene.reduced
      const n = still ? Math.floor(t / STILL_MS) : Math.floor(t / MOVE_MS)
      const local = still ? Infinity : t - n * MOVE_MS
      const leaveFrom = MOVE_MS - LEAVE_MS
      const { c0, r0, board } = placeOf(n)

      // The clock's own board of trace dots, the rest of the screen dark.
      for (let r = board.r0; r < board.r0 + boardH; r++) {
        for (let c = board.c0; c < board.c0 + boardW; c++) set(c, r, dot(TRACE))
      }

      /** A dot's ink as the clock comes in and goes out around it (x, y in cells from the numerals' corner). */
      const life = (level: number, x: number, y: number) => {
        if (still) return level
        const order = 0.6 * (x / NUMERALS_W) + 0.4 * hash(x + n, y)
        if (local < order * RESOLVE_MS) return TRACE
        if (local >= leaveFrom) {
          const out = local - leaveFrom - hash(y, x + n) * (LEAVE_MS - STEP_MS * 3)
          if (out >= 0) return Math.max(TRACE, level - 1 - Math.floor(out / STEP_MS))
        }
        return level
      }
      /** One numeral dot: DOT x DOT cells, switching together. */
      const big = (x: number, y: number, level: number, accent = false) => {
        const v = life(level, x * DOT, y * DOT)
        for (let dy = 0; dy < DOT; dy++) for (let dx = 0; dx < DOT; dx++) set(c0 + x * DOT + dx, r0 + y * DOT + dy, dot(v, accent && v > TRACE))
      }

      for (const slot of SLOTS) {
        for (let y = 0; y < NUMERAL_H; y++) {
          if (slot.kind === 'colon') {
            if (COLON[y] === '#') big(slot.x, y, still ? FULL : breath(t), true)
            continue
          }
          for (let x = 0; x < NUMERAL_W; x++) {
            const on = numeralDot(shownDigits[slot.i]!, x, y)
            const was = numeralDot(prevDigits[slot.i]!, x, y)
            let level = on ? FULL : TRACE
            if (on !== was && !still) {
              // Dot by dot, in no particular order: off, a beat at half, on (or back).
              const at = t - changedAt - hash(slot.i * 7 + x, y) * (SWITCH_MS - STEP_MS)
              if (at < 0) level = was ? FULL : TRACE
              else if (at < STEP_MS) level = HALF
            }
            big(slot.x + x, y, level)
          }
        }
      }

      // AM or PM, small, at the numerals' foot.
      if (twelve) {
        const word = pm ? 'PM' : 'AM'
        for (let i = 0; i < 2; i++) {
          const x = NUMERALS_W + 2 + i
          set(c0 + x, r0 + NUMERALS_H - 1, glyph(word[i]!, life(HALF, x, NUMERALS_H - 1)))
        }
      }

      // The date and KRYOTO, then the game played last: a quieter block, centred under the numerals.
      if (!scene.thumb) {
        const last = scene.lastPlayed()
        const lines: [string, number][] = [
          [dateLine(now), HALF],
          [last ? `LAST PLAYED: ${saverText(last, Math.max(8, NUMERALS_W - 13))}` : '', LOW],
        ]
        lines.forEach(([line, ink], k) => {
          const y = NUMERALS_H + TEXT_GAP + k * 2
          const start = Math.floor((NUMERALS_W - line.length) / 2)
          for (let i = 0; i < line.length; i++) {
            const x = start + i
            const ch = line[i]!
            if (ch === ' ') continue
            // KRYOTO, and the slashes before it, a step quieter than the date.
            const level = k === 0 && i >= line.length - 9 ? LOW : ink
            const lit = life(level, Math.max(0, Math.min(NUMERALS_W - 1, x)), y)
            set(c0 + x, r0 + y, lit === TRACE ? dot(TRACE) : glyph(ch, lit))
          }
        })
      }
    },
  }
}

/** With motion reduced, the still changes when the minute does. */
const nextMinute = (now: Date) => 60_000 - (now.getSeconds() * 1000 + now.getMilliseconds()) + 50

export const CLOCK_SAVER: VariantSpec = { id: 'clock', name: 'Clock', scale: SCALE, stillFor: nextMinute, make }
