// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { FRIENDS_EVERY_MS, FRIENDS_REFUSED_MS, friendsNext } from './friends-poll.ts'

test('an account in Anonymous mode is not asked every half minute', () => {
  // 403 anonymous_mode: the same answer every time.
  const next = friendsNext(403, 0)
  assert.equal(next.delay, FRIENDS_REFUSED_MS)
  assert.ok(next.delay >= 60 * FRIENDS_EVERY_MS)
})

test('a feature not on for the account (404) waits the same', () => {
  assert.equal(friendsNext(404, 240_000).delay, FRIENDS_REFUSED_MS)
  assert.equal(friendsNext(404, 240_000).wait, 0)
})

test('other failures back off and stop growing at fifteen minutes', () => {
  let wait = 0
  const delays: number[] = []
  for (let i = 0; i < 6; i++) {
    const next = friendsNext(502, wait)
    wait = next.wait
    delays.push(next.delay / 60_000)
  }
  assert.deepEqual(delays, [1, 2, 4, 8, 15, 15])
  // No connection at all counts the same.
  assert.equal(friendsNext(0, 0).delay, 60_000)
})
