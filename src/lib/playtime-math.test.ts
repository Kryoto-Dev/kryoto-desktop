// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { LibraryGame } from './library'
import { countedHere, playtimeGap, sessionStart, syncKey } from './playtime-math.ts'

const game = (g: Partial<LibraryGame>) => ({ playtimeSeconds: 0, carrySends: [], ...g }) as LibraryGame

test('only what this PC has beyond the account and what is on its way is sent (KB-0261)', () => {
  // BeamNG on 2026-11-09: 8975 s here, 6189 s on kryo.to.
  assert.equal(playtimeGap(8975, 6189, 0), 2786)
  // A session still on its way is not counted twice.
  assert.equal(playtimeGap(8975, 6189, 2700), 86)
  // Under a minute, or the account already ahead: nothing.
  assert.equal(playtimeGap(8975, 6189, 2750), 0)
  assert.equal(playtimeGap(100, 500, 0), 0)
})

test('the same difference has the same key, a new one a new key', () => {
  assert.equal(syncKey('beamng-drive', 8975, 6189), syncKey('beamng-drive', 8975.4, 6189))
  assert.notEqual(syncKey('beamng-drive', 8975, 6189), syncKey('beamng-drive', 9100, 6189))
  assert.ok(syncKey('x'.repeat(200), 1, 2).length <= 80)
  assert.match(syncKey('a b/c', 1, 2), /^[a-zA-Z0-9-]+$/)
})

test("only games Kryoto counts itself, not while they run, branches summed per kryo.to game", () => {
  const rows = countedHere(
    [
      game({ id: 'a', slug: 'gorebox', playtimeSeconds: 600 }),
      game({ id: 'b', slug: 'gorebox', playtimeSeconds: 300, carrySends: [{ seconds: 120 }] as LibraryGame['carrySends'] }),
      game({ id: 'c', slug: 'hades', playtimeSeconds: 900 }),
      game({ id: 'd', slug: 'portal', playtimeSeconds: 900, launcher: 'steam' }),
      game({ id: 'e', playtimeSeconds: 900 }),
      game({ id: 'f', slug: 'celeste', playtimeSeconds: 0 }),
    ],
    ['c'],
  )
  assert.deepEqual(rows, [{ id: 'a', slug: 'gorebox', local: 900, carrying: 120 }])
})

test('a session started here is not counted from the last one (KB-0344)', () => {
  const now = 1_800_000_000
  // Play pressed: lastPlayed is still the previous session's, two hours back.
  assert.equal(sessionStart(now - 7200, now), now)
  // Rust stamped the new start a moment after the press: that one counts.
  assert.equal(sessionStart(now + 3, now), now + 3)
  // Already running when the view opened: its own start counts, however long ago.
  assert.equal(sessionStart(now - 3600, null), now - 3600)
  assert.equal(sessionStart(null, null), null)
  assert.equal(sessionStart(null, now), now)
})
