import { dot, FAINT, FULL, glyph, HALF, hash, LOW, STILL_MS, TRACE, type Scene, type VariantSpec } from './engine.ts'
import { saverText } from './titles.ts'

/**
 * Rain: a few sparse columns of Kryoto's ASCII stepping down the trace-dot
 * grid, each at its own slow pace, in one ink. Now and then a column comes
 * to rest as K// or a game from the library, read downwards, holds for a
 * breath in the reader's accent, and goes out letter by letter.
 */

/** Columns falling at once, per hundred columns of screen. */
const DROPS_PER_100 = 7
/** Rows a second, slowest and fastest. */
const SPEED_MIN = 0.9
const SPEED_MAX = 2.2
/** Trail length in cells, shortest and longest. */
const TRAIL_MIN = 6
const TRAIL_MAX = 14
/** The quiet before a column starts again somewhere else, at most. */
const REST_MAX_MS = 6_000
/** A column resolves into a word this often, holds it this long, then lets it go. */
const WORD_EVERY_MS = 20_000
const WORD_HOLD_MS = 6_500
const WORD_LEAVE_MS = 1_400
/** A word's column falls from this many rows above it, at this many rows a second. */
const WORD_LEAD = 8
const WORD_SPEED = 3.2
/** A trail glyph changes now and then: this often, at the fastest. */
const MUTATE_MS = 1_400
/** Cells this much larger than the default, so a glyph reads from the couch. */
const SCALE = 1.6
/** Kryoto's ASCII, and its letters. */
export const RAIN_GLYPHS = '/\\.:_-=<>+*KRYOTO'

type Drop = {
  col: number
  start: number
  speed: number
  len: number
  seed: number
  /** A word this column comes to rest as, top row first. */
  word: string | null
  wordRow: number
}

function make(scene: Scene) {
  const grid = scene.grid
  const { view, cols, rows } = grid
  const rand = scene.rand
  const h = view.r1 - view.r0
  const count = Math.max(2, Math.round(((view.c1 - view.c0) * DROPS_PER_100) / 100))
  const drops: Drop[] = []
  let words = 0

  const busy = (col: number) => drops.some((d) => Math.abs(d.col - col) < 2)
  const spawn = (t: number, word: string | null = null): Drop => {
    let col = view.c0 + Math.floor(rand() * (view.c1 - view.c0))
    for (let tries = 0; tries < 8 && busy(col); tries++) col = view.c0 + Math.floor(rand() * (view.c1 - view.c0))
    const len = word ? word.length : 0
    return {
      col,
      start: t + (word ? 0 : rand() * REST_MAX_MS),
      speed: word ? WORD_SPEED : SPEED_MIN + rand() * (SPEED_MAX - SPEED_MIN),
      len: TRAIL_MIN + Math.floor(rand() * (TRAIL_MAX - TRAIL_MIN + 1)),
      seed: rand() * 1000,
      word,
      wordRow: view.r0 + 3 + Math.floor(rand() * Math.max(1, h - len - 6)),
    }
  }
  for (let i = 0; i < count; i++) {
    const d = spawn(0)
    d.start = -rand() * 12_000 // already falling when it starts
    drops.push(d)
  }

  const nextWord = () => {
    const titles = scene.titles()
    const pick = words % 3 === 0 || !titles.length ? 'K//' : titles[Math.floor(words / 3) % titles.length]!
    words++
    return saverText(pick, Math.max(3, Math.min(22, h - 8)))
  }

  const set = (c: number, r: number, v: number) => {
    if (c >= 0 && r >= 0 && c < cols && r < rows) grid.cells[r * cols + c] = v
  }
  const pick = (d: Drop, r: number, t: number) => {
    const mutate = Math.floor((t + hash(d.seed, r) * MUTATE_MS * 4) / (MUTATE_MS * (2 + hash(r, d.seed) * 3)))
    return RAIN_GLYPHS[Math.floor(hash(d.seed + mutate, r) * RAIN_GLYPHS.length)]!
  }

  let wordAt = WORD_EVERY_MS * 0.4

  return {
    frame(t: number) {
      grid.cells.fill(dot(FAINT))
      if (scene.reduced) {
        // A still: a few columns where they are, and one word at rest.
        const k = Math.floor(t / STILL_MS)
        for (let i = 0; i < count; i++) {
          const col = view.c0 + Math.floor(hash(i, k) * (view.c1 - view.c0))
          const head = view.r0 + Math.floor(hash(k, i) * h)
          for (let j = 0; j < 8; j++) set(col, head - j, glyph(RAIN_GLYPHS[Math.floor(hash(i + j, k) * RAIN_GLYPHS.length)]!, j === 0 ? HALF : LOW))
        }
        const word = saverText(scene.titles()[k % Math.max(1, scene.titles().length)] ?? 'K//', 22)
        const col = view.c0 + 4 + Math.floor(hash(k, 99) * (view.c1 - view.c0 - 8))
        for (let i = 0; i < word.length; i++) set(col, view.r0 + 4 + i, glyph(word[i]!, HALF, true))
        return
      }

      if (t >= wordAt && !scene.thumb) {
        wordAt = t + WORD_EVERY_MS
        drops.push(spawn(t, nextWord()))
      }

      for (let i = drops.length - 1; i >= 0; i--) {
        const d = drops[i]!
        const fallen = ((t - d.start) / 1000) * d.speed
        if (fallen < 0) continue
        // A word's column starts a little above it, so it arrives within a breath, not a minute.
        const top = d.word ? Math.max(view.r0 - 1, d.wordRow - WORD_LEAD) : view.r0 - 1
        const head = top + Math.floor(fallen)
        const wordEnd = d.word ? d.wordRow + d.word.length - 1 : Infinity
        // Once a word's column has reached its end, the trail above drains away.
        const landed = d.word && head >= wordEnd
        const holdFrom = d.word ? d.start + ((wordEnd - top + 1) / d.speed) * 1000 : Infinity

        for (let k = 0; k < d.len + (d.word?.length ?? 0); k++) {
          const r = head - k
          if (r > wordEnd || r < view.r0 - 1) continue
          const inWord = d.word && r >= d.wordRow && r <= wordEnd
          if (inWord) continue
          if (k >= d.len) continue
          const level = k === 0 ? FULL : k <= 2 ? HALF : k <= 5 ? LOW : TRACE
          set(d.col, r, glyph(pick(d, r, t), level))
        }

        if (d.word) {
          const reached = Math.min(head, wordEnd)
          for (let r = d.wordRow; r <= reached; r++) {
            const ch = d.word[r - d.wordRow]!
            if (ch === ' ') continue
            let v = glyph(ch, r === head ? FULL : HALF)
            if (landed) {
              const since = t - holdFrom
              v = glyph(ch, FULL, true)
              if (since >= WORD_HOLD_MS) {
                const out = since - WORD_HOLD_MS - hash(r, d.seed) * (WORD_LEAVE_MS - 300)
                if (out >= 300) v = dot(FAINT)
                else if (out >= 150) v = glyph(ch, LOW)
                else if (out >= 0) v = glyph(ch, HALF)
              }
            }
            set(d.col, r, v)
          }
          if (t - holdFrom > WORD_HOLD_MS + WORD_LEAVE_MS) drops.splice(i, 1)
          continue
        }

        // Gone off the bottom: start again somewhere else, after a rest.
        if (head - d.len > view.r1) drops[i] = spawn(t)
      }
    },
  }
}

export const RAIN_SAVER: VariantSpec = { id: 'rain', name: 'Rain', scale: SCALE, make }
