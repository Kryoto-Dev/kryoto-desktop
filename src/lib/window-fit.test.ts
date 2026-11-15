// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { fitWindow, SCREEN_MARGIN } from './window-fit.ts'

const MAIN = { width: 1280, height: 800 }
const MIN = { width: 1000, height: 640 }

test('a 1920 by 1080 screen at 150% keeps the whole client on it (KB-0321)', () => {
  // 1280 by 720 logical, less a 48 px taskbar at 150%.
  const room = { width: 1280, height: 688 }
  const { size, min } = fitWindow(MAIN, MIN, room)
  assert.equal(size.width, 1280 - 2 * SCREEN_MARGIN)
  assert.equal(size.height, 688 - 2 * SCREEN_MARGIN)
  assert.ok(min.width <= size.width && min.height <= size.height)
})

test('a size saved on a bigger screen is brought inside a smaller one', () => {
  const { size } = fitWindow({ width: 2400, height: 1300 }, MIN, { width: 1536, height: 816 })
  assert.deepEqual(size, { width: 1536 - 2 * SCREEN_MARGIN, height: 816 - 2 * SCREEN_MARGIN })
})

test('the smallest size gives way on a screen smaller than it', () => {
  const { size, min } = fitWindow(MAIN, MIN, { width: 960, height: 540 })
  assert.deepEqual(min, size)
})

test('room to spare, or an unknown screen: the size as asked', () => {
  assert.deepEqual(fitWindow(MAIN, MIN, { width: 2560, height: 1400 }), { size: MAIN, min: MIN })
  assert.deepEqual(fitWindow(MAIN, MIN, null), { size: MAIN, min: MIN })
})
