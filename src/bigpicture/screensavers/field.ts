import { glyph, hash, HALF, LOW, OFF, STILL_MS, TRACE, type Scene, type VariantSpec } from './engine.ts'
import { saverText } from './titles.ts'

/**
 * Field: the start screen's own scene (boot/SplashScene.tsx, which the host
 * draws underneath when asked), with the names of games from the library
 * surfacing in the dark one at a time: each letter scrambles in and settles,
 * the name holds, and it goes out letter by letter. Above or below the mark,
 * never across it.
 */

/** A letter arrives this long after the one before it, and scrambles this long. */
const LETTER_MS = 90
const SCRAMBLE_MS = 420
/** The scramble changes glyph this often (hard steps). */
const SCRAMBLE_STEP_MS = 70
/** A name holds this long once complete, then goes out over this long. */
const HOLD_MS = 6_500
const LEAVE_MS = 1_600
/** The start screen's own opening plays first. */
const FIRST_MS = 3_000
/** Cells this much larger than the default, so a name reads from the couch. */
const SCALE = 1.3
/** The dark between two names. */
const GAP_MS = 2_500
/** The longest name, in letters. */
const MAX_LETTERS = 28
/** What a letter passes through on its way in: Kryoto's ASCII. */
export const SCRAMBLE_GLYPHS = '/\\.:_-=<>+*'

function make(scene: Scene) {
  const grid = scene.grid
  const { view, cols } = grid
  scene.underlay('splash')
  const offset = Math.floor(scene.rand() * 1000)

  /** The k-th name to show: where, and when. */
  const plan = (k: number, titles: readonly string[]) => {
    const name = saverText(titles[(k + offset) % titles.length]!, MAX_LETTERS)
    const span = name.length * LETTER_MS + SCRAMBLE_MS + HOLD_MS + LEAVE_MS
    const w = view.c1 - view.c0
    const h = view.r1 - view.r0
    // The mark sits a little above the middle: names go in the band above it or the one below.
    // Centred across the screen, in a band well clear of the mark and of the edges.
    const above = hash(k, offset) < 0.5
    const band = above ? [0.16, 0.22] : [0.72, 0.78]
    const r = view.r0 + Math.round(h * (band[0]! + hash(offset, k) * (band[1]! - band[0]!)))
    const c = view.c0 + Math.floor((w - name.length) / 2)
    return { name, span, c, r }
  }

  return {
    frame(t: number) {
      grid.cells.fill(OFF)
      const titles = scene.titles()
      if (!titles.length || scene.thumb) return
      if (scene.reduced) {
        const p = plan(Math.floor(t / STILL_MS), titles)
        for (let i = 0; i < p.name.length; i++) put(p.c + i, p.r, glyph(p.name[i]!, HALF))
        return
      }
      // Which name, and how far into it.
      const per = MAX_LETTERS * LETTER_MS + SCRAMBLE_MS + HOLD_MS + LEAVE_MS + GAP_MS
      if (t < FIRST_MS) return
      const k = Math.floor((t - FIRST_MS) / per)
      const local = t - FIRST_MS - k * per
      const p = plan(k, titles)
      const leaveFrom = p.name.length * LETTER_MS + SCRAMBLE_MS + HOLD_MS
      for (let i = 0; i < p.name.length; i++) {
        const ch = p.name[i]!
        if (ch === ' ') continue
        const at = local - i * LETTER_MS
        if (at < 0) continue
        let v: number
        if (at < SCRAMBLE_MS) {
          const step = Math.floor(at / SCRAMBLE_STEP_MS)
          v = glyph(SCRAMBLE_GLYPHS[Math.floor(hash(i + k * 31, step) * SCRAMBLE_GLYPHS.length)]!, LOW)
        } else v = glyph(ch, HALF)
        if (local >= leaveFrom) {
          const out = local - leaveFrom - hash(i, k + 7) * (LEAVE_MS - 300)
          if (out >= 300) continue
          if (out >= 150) v = glyph(ch, TRACE)
          else if (out >= 0) v = glyph(ch, LOW)
        }
        put(p.c + i, p.r, v)
      }
    },
  }

  function put(c: number, r: number, v: number) {
    if (c >= 0 && r >= 0 && c < cols && r < grid.rows) grid.cells[r * cols + c] = v
  }
}

export const FIELD_SAVER: VariantSpec = { id: 'field', name: 'Field', scale: SCALE, make }
