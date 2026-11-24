// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { capDelay, capReady, dot, FULL, glyph, HALF, isGlyph, levelOf, LOW, pitchFor, seeded, TRACE } from './engine.ts'
import { MARK_CELLS, MARK_COLS, MARK_DOTS, MARK_ROWS } from './mark-data.ts'
import { cellInk } from './matrix-ink.ts'
import { clockDigits, COLON, NUMERAL_H, NUMERAL_W, NUMERALS } from './numerals.ts'
import { shuffleOrder } from './shuffle.ts'
import { lastPlayedTitle, saverText, saverTitles } from './titles.ts'

test('every numeral is five dots across and seven down, and no two are alike', () => {
  const digits = '0123456789'
  for (const d of digits) {
    const rows = NUMERALS[d]!
    assert.equal(rows.length, NUMERAL_H, d)
    for (const row of rows) assert.match(row, /^[#.]{5}$/, d)
    assert.equal(rows[0]!.length, NUMERAL_W)
    // A dot in the top and bottom rows: a row of numerals reads as one line of type.
    assert.ok(rows[0]!.includes('#') && rows[NUMERAL_H - 1]!.includes('#'), `${d} reaches top and bottom`)
  }
  const shapes = new Set(Array.from(digits, (d) => NUMERALS[d]!.join('')))
  assert.equal(shapes.size, 10)
  assert.equal(COLON.length, NUMERAL_H)
  assert.equal(COLON.filter((c) => c === '#').length, 2)
})

test('the clock reads the time in 24 hours, and in 12 without jumping sideways', () => {
  const at = (h: number, m: number) => new Date(2026, 10, 23, h, m)
  assert.deepEqual(clockDigits(at(9, 41), false), { digits: '0941', pm: false })
  assert.deepEqual(clockDigits(at(21, 5), false), { digits: '2105', pm: true })
  assert.deepEqual(clockDigits(at(9, 41), true), { digits: ' 941', pm: false })
  assert.deepEqual(clockDigits(at(0, 0), true), { digits: '1200', pm: false })
  assert.deepEqual(clockDigits(at(12, 30), true), { digits: '1230', pm: true })
  assert.deepEqual(clockDigits(at(23, 59), true), { digits: '1159', pm: true })
})

test("the mark's dots are the Discord recipe's: every 2 x 4 block agrees with the character it was cut from", () => {
  assert.equal(MARK_COLS, 40)
  assert.equal(MARK_ROWS, 24)
  for (const row of MARK_DOTS) assert.match(row, /^[#+.]{40}$/)
  assert.equal(MARK_CELLS.length, 6)
  for (const row of MARK_CELLS) assert.match(row, /^[#+.]{20}$/)
  for (let r = 0; r < 6; r++) {
    for (let c = 0; c < 20; c++) {
      let full = 0
      for (let y = 0; y < 4; y++) for (let x = 0; x < 2; x++) if (MARK_DOTS[r * 4 + y]![c * 2 + x] === '#') full++
      assert.equal(full >= 4, MARK_CELLS[r]![c] === '#', `character ${c},${r}`)
    }
  }
})

test("the wall's matrices move as ui/Matrix.tsx does: the orbit's head and two-step tail", () => {
  // Cell 0 leads the ring: full at the start, half a step later, then low, then a trace.
  assert.equal(cellInk('busy', 0, 0), FULL)
  assert.equal(cellInk('busy', 0, 110), HALF)
  assert.equal(cellInk('busy', 0, 220), LOW)
  assert.equal(cellInk('busy', 0, 330), TRACE)
  // The middle is not on the ring: it keeps its still ink, a trace.
  assert.equal(cellInk('busy', 4, 500), TRACE)
  // Sync has two heads, opposite.
  assert.equal(cellInk('sync', 0, 0), FULL)
  assert.equal(cellInk('sync', 8, 440), FULL)
})

test('cells pack a dot or a glyph, an ink and the accent', () => {
  assert.equal(levelOf(dot(HALF)), HALF)
  assert.equal(isGlyph(dot(FULL)), false)
  assert.equal(isGlyph(glyph('K', LOW)), true)
  assert.equal(levelOf(glyph('K', LOW, true)), LOW)
  assert.notEqual(glyph('K', FULL, true), glyph('K', FULL))
  assert.equal(glyph(' ', FULL), dot(TRACE), 'a space shows the grid')
  assert.equal(glyph('K', 0), 0)
})

test('frames come no more often than the cap', () => {
  // Asked a display frame early, then held until the cap allows.
  assert.equal(capDelay(1000, 1000, 20), 50 - 16)
  assert.equal(capDelay(1000, 1100, 20), 0)
  assert.equal(capReady(1000, 1030, 20), false)
  assert.equal(capReady(1000, 1049.5, 20), true)
  // Simulated: a 60 Hz display, frames drawn only when the cap is ready.
  let last = -Infinity
  let drawn = 0
  for (let now = 0; now < 10_000; now += 1000 / 60) {
    if (capReady(last, now, 20)) {
      last = now
      drawn++
    }
  }
  assert.ok(drawn <= 201, `${drawn} frames in 10 s`)
  assert.ok(drawn >= 150, `${drawn} frames in 10 s`)
})

test('cells are the same share of any 16:9 screen, in whole pixels', () => {
  assert.equal(pitchFor(1920, 1080), 24)
  assert.equal(pitchFor(3840, 2160), 48)
  assert.equal(pitchFor(2560, 1080), 24, 'an ultrawide screen goes by its height')
  assert.equal(pitchFor(1920, 1080, 1.45), 35)
  assert.ok(Number.isInteger(pitchFor(1366, 768)))
})

test('shuffle plays every one before any comes back, never the same twice in a row', () => {
  const ids = ['mark', 'clock', 'field', 'rain', 'snake']
  for (let seed = 1; seed < 40; seed++) {
    const order = shuffleOrder(ids, 6, seeded(seed))
    assert.equal(order.length, 30)
    for (let round = 0; round < 6; round++) assert.deepEqual(order.slice(round * 5, round * 5 + 5).sort(), [...ids].sort())
    for (let i = 1; i < order.length; i++) assert.notEqual(order[i], order[i - 1], `seed ${seed} at ${i}`)
  }
})

test('titles leave out hidden games, and adult ones unless the reader shows them', () => {
  const games = [
    { title: 'Celeste', lastPlayed: 100 },
    { title: 'Secret Game', hidden: true, lastPlayed: 500 },
    { title: 'Adult Game', nsfw: true, lastPlayed: 400 },
    { title: 'Hades', lastPlayed: 300 },
    { title: 'Terraria', lastPlayed: null },
  ]
  assert.deepEqual(saverTitles(games, { show: false, hide: false }), ['Hades', 'Celeste', 'Terraria'])
  assert.deepEqual(saverTitles(games, { show: true, hide: true }), ['Hades', 'Celeste', 'Terraria'], 'hiding adult games wins over showing their art')
  assert.deepEqual(saverTitles(games, { show: true, hide: false }), ['Adult Game', 'Hades', 'Celeste', 'Terraria'])
  assert.equal(lastPlayedTitle(games, { show: false, hide: false }), 'Hades', 'never the hidden one played after it')
  assert.equal(lastPlayedTitle([{ title: 'Never played', lastPlayed: null }], { show: false, hide: false }), null)
})

test('titles are set in capitals, plain ASCII, and cut to fit', () => {
  assert.equal(saverText('Pokémon™ Legends: Arceus', 40), 'POKEMON LEGENDS: ARCEUS')
  assert.equal(saverText('Baldur’s Gate 3', 40), "BALDUR'S GATE 3")
  assert.equal(saverText('The Elder Scrolls V: Skyrim Special Edition', 16), 'THE ELDER SCR...')
  assert.ok(saverText('A very long game title indeed', 10).length <= 10)
})
