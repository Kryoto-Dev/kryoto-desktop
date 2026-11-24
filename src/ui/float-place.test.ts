// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { placeFloat } from './float-place.ts'

const at = (x: number, y: number) => ({ left: x, right: x, top: y, bottom: y })

test('a right-click menu taller than a small window fits inside it and scrolls', () => {
  // The smallest client window at 150% text: 667 by 427 CSS px; a game's menu with collections, 610 px.
  const view = { width: 667, height: 427 }
  const p = placeFloat({ anchor: at(300, 200), width: 240, height: 610, view, gap: 0, cover: true })
  assert.equal(p.maxHeight, 427 - 16)
  assert.equal(p.top, 8)
  assert.ok(p.top + p.maxHeight <= view.height - 8)
})

test('opened near the bottom, it goes up and stays inside', () => {
  const view = { width: 1280, height: 720 }
  const p = placeFloat({ anchor: at(900, 700), width: 240, height: 400, view, gap: 0, cover: true })
  assert.equal(p.maxHeight, 400)
  assert.ok(p.top >= 8 && p.top + p.maxHeight <= 712)
})

test('opened at the right edge, it moves left into the window', () => {
  const view = { width: 1000, height: 640 }
  const p = placeFloat({ anchor: at(990, 100), width: 260, height: 200, view, gap: 0 })
  assert.equal(p.left, 1000 - 260 - 8)
})

test('a dropdown keeps to its side of the control and scrolls there', () => {
  const view = { width: 1000, height: 400 }
  const control = { left: 100, right: 300, top: 120, bottom: 150 }
  const p = placeFloat({ anchor: control, width: 200, height: 500, view, gap: 4 })
  // More room below (400 - 150 - 12) than above (120 - 12).
  assert.equal(p.top, 154)
  assert.equal(p.maxHeight, 400 - 150 - 4 - 8)
})

test('one that fits opens under its control at its own height', () => {
  const p = placeFloat({ anchor: { left: 10, right: 110, top: 10, bottom: 40 }, width: 200, height: 120, view: { width: 800, height: 600 } })
  assert.deepEqual(p, { top: 46, left: 10, maxHeight: 120 })
})
