// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { MISSES, pointerWatch } from './pointer-watch.ts'

test('one ask with no answer (a UAC prompt) leaves the clicks going through', () => {
  const watch = pointerWatch()
  assert.equal(watch.missed(), false)
  watch.answered()
  for (let i = 1; i < MISSES; i++) assert.equal(watch.missed(), false, `miss ${i}`)
})

test('a run of misses takes the clicks, and the next answer gives them back', () => {
  const watch = pointerWatch()
  for (let i = 1; i < MISSES; i++) watch.missed()
  assert.equal(watch.missed(), true)
  assert.equal(watch.missed(), true, 'and stays so while it lasts')
  watch.answered()
  assert.equal(watch.missed(), false, 'counted again from the start')
})
