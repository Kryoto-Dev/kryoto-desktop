import { dot, FULL, glyph, HALF, hash, LOW, OFF, STILL_MS, TRACE, type Scene, type VariantSpec } from './engine.ts'
import { clockDigits, COLON, NUMERAL_H, NUMERAL_W, numeralDot, systemTwelveHour } from './numerals.ts'
import { saverText } from './titles.ts'

/**
 * Clock: the time in matrix-dot numerals, the date and KRYOTO small beneath,
 * and the game played last. A numeral changes by its dots switching, one by
 * one, never by sliding; the colon blinks in hard steps, in the reader's
 * accent. Every minute the whole clock goes out dot by dot and comes back
 * somewhere else. 12 or 24 hours as the system shows them.
 */

/** Cells this much larger than the other screensavers': a clock is read from the couch. */
const SCALE = 1.45
/** A new place this often. */
const MOVE_MS = 60_000
/** Going out and coming in, dot by dot. */
const LEAVE_MS = 1_400
const RESOLVE_MS = 1_800
/** A numeral's dots switch over this long when the minute turns. */
const SWITCH_MS = 700
/** One dot's switch: a beat at half ink between off and on. */
const STEP_MS = 90
/** The colon: lit, then a trace, this long each. */
const BLINK_MS = 1_000
/** Gaps, in cells: between numerals, around the colon, above the lines of text. */
const GAP = 1
const TEXT_GAP = 2
/** The board of trace dots around the clock reaches this far past it, in cells. */
const PANEL_SIDE = 7
const PANEL_EDGE = 2

type Layout = { width: number; height: number; ampm: number }

function measure(twelve: boolean): Layout {
  const digits = 4 * NUMERAL_W + 3 * GAP + 1 + 2 * GAP - GAP
  const ampm = twelve ? 3 : 0
  return { width: digits + ampm, height: NUMERAL_H + TEXT_GAP + 3, ampm }
}

/** The numerals' columns, left to right: four numerals and the colon. */
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

function dateLine(now: Date): string {
  const day = new Intl.DateTimeFormat(undefined, { weekday: 'short', day: 'numeric', month: 'short' }).format(now)
  return `${saverText(day, 18)} // KRYOTO`
}

function make(scene: Scene) {
  const grid = scene.grid
  const { view, cols, rows } = grid
  const twelve = systemTwelveHour()
  const box = measure(twelve)
  const jitter = scene.rand()

  const placeOf = (n: number) => {
    if (scene.thumb) {
      return { c0: Math.round((view.c0 + view.c1 - box.width) / 2), r0: Math.round((view.r0 + view.r1 - box.height) / 2) }
    }
    // The lines of text may reach five cells past the numerals on each side.
    const roomC = Math.max(0, view.c1 - view.c0 - box.width - 16)
    const roomR = Math.max(0, view.r1 - view.r0 - box.height - 6)
    // Anywhere in the middle four fifths: it wanders, but never hugs an edge.
    return {
      c0: view.c0 + 8 + Math.floor((0.1 + 0.8 * hash(n, jitter * 97)) * roomC),
      r0: view.r0 + 3 + Math.floor((0.1 + 0.8 * hash(jitter * 53, n)) * roomR),
    }
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
      const { c0, r0 } = placeOf(n)

      // The clock's own board of trace dots, the rest of the screen dark.
      const side = scene.thumb ? 2 : PANEL_SIDE
      for (let r = r0 - PANEL_EDGE; r < r0 + box.height + PANEL_EDGE - 1; r++) {
        for (let c = c0 - side; c < c0 + box.width + side; c++) set(c, r, dot(TRACE))
      }

      /** A dot's ink as the clock comes in and goes out around it. */
      const life = (level: number, x: number, y: number) => {
        if (still) return level
        const order = 0.6 * (x / box.width) + 0.4 * hash(x + n, y)
        if (local < order * RESOLVE_MS) return TRACE
        if (local >= leaveFrom) {
          const out = local - leaveFrom - hash(y, x + n) * (LEAVE_MS - STEP_MS * 3)
          if (out >= 0) return Math.max(TRACE, level - 1 - Math.floor(out / STEP_MS))
        }
        return level
      }

      for (const slot of SLOTS) {
        for (let y = 0; y < NUMERAL_H; y++) {
          if (slot.kind === 'colon') {
            if (COLON[y] !== '#') continue
            const on = still || Math.floor(t / BLINK_MS) % 2 === 0
            const level = life(on ? FULL : TRACE, slot.x, y)
            set(c0 + slot.x, r0 + y, dot(level, level === FULL))
            continue
          }
          for (let x = 0; x < NUMERAL_W; x++) {
            const now1 = numeralDot(shownDigits[slot.i]!, x, y)
            const was = numeralDot(prevDigits[slot.i]!, x, y)
            let level = now1 ? FULL : TRACE
            if (now1 !== was && !still) {
              // Dot by dot, in no particular order: off, a beat at half, on (or back).
              const at = t - changedAt - hash(slot.i * 7 + x, y) * (SWITCH_MS - STEP_MS)
              if (at < 0) level = was ? FULL : TRACE
              else if (at < STEP_MS) level = HALF
            }
            set(c0 + slot.x + x, r0 + y, dot(life(level, slot.x + x, y)))
          }
        }
      }

      // AM or PM, small, at the numerals' foot.
      if (box.ampm) {
        const word = pm ? 'PM' : 'AM'
        for (let i = 0; i < 2; i++) set(c0 + box.width - 2 + i, r0 + NUMERAL_H - 1, glyph(word[i]!, life(HALF, box.width - 2 + i, NUMERAL_H - 1)))
      }

      // The date and KRYOTO, then the game played last, centred under the numerals.
      if (!scene.thumb) {
        const line1 = dateLine(now)
        const last = scene.lastPlayed()
        const line2 = last ? `LAST PLAYED: ${saverText(last, Math.max(8, box.width + 10 - 13))}` : ''
        const lines: [string, number][] = [
          [line1, HALF],
          [line2, LOW],
        ]
        lines.forEach(([line, ink], k) => {
          const y = NUMERAL_H + TEXT_GAP + k * 2
          const start = Math.round((box.width - line.length) / 2)
          for (let i = 0; i < line.length; i++) {
            const x = start + i
            const ch = line[i]!
            // KRYOTO, and the slashes before it, a step quieter than the date.
            const level = k === 0 && i >= line.length - 9 ? LOW : ink
            if (ch === ' ') continue
            const lit = life(level, Math.max(0, Math.min(box.width - 1, x)), y)
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
