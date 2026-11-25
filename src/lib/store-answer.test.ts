// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { storeAnswer } from './store-answer.ts'

test("a refusal is said in kryo.to's own words", () => {
  // Add to collection, as kryo.to once refused it.
  assert.throws(() => storeAnswer({ ok: false, status: 400, data: { error: 'A game slug is required.' } }), { message: 'A game slug is required.' })
})

test('a refusal with nothing to say gives its status, and no answer at all says so', () => {
  assert.throws(() => storeAnswer({ ok: false, status: 404, data: {} }), { message: 'kryo.to answered 404.' })
  assert.throws(() => storeAnswer({ ok: false, status: 502, data: { error: '  ' } }), { message: 'kryo.to answered 502.' })
  assert.throws(() => storeAnswer({ ok: false, status: 500, data: { error: { code: 1 } } }), { message: 'kryo.to answered 500.' })
  assert.throws(() => storeAnswer({ ok: false, status: 0, data: null }), { message: 'Could not reach kryo.to.' })
})

test('an answer gives its data, or nothing when it had no body', () => {
  assert.deepEqual(storeAnswer({ ok: true, status: 200, data: { ok: true } }), { ok: true })
  assert.deepEqual(storeAnswer({ ok: true, status: 204, data: null }), {})
})
