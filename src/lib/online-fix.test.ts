// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { onlineFixView, versionLine, type OnlineFixLookup, type OnlineFixState } from './online-fix.ts'

const lookup = (over: Partial<OnlineFixLookup> = {}): OnlineFixLookup => ({ status: 'found', page: 'https://online-fix.me/games/x/1-g.html', file: 'G_Fix_Repair_Steam_Generic.rar', size: '11M', madeFor: null, newerNeeded: false, detail: null, ...over })
const state = (over: Partial<OnlineFixState> = {}): OnlineFixState => ({ blocked: null, carried: false, applied: false, sevenZip: true, installed: '1.0.9', lookup: null, ...over })

test('while online-fix.me is asked, nothing is claimed', () => {
  assert.equal(onlineFixView(state()), null)
})

test('a fix online-fix.me has is offered, with the version it is made for', () => {
  const v = onlineFixView(state({ lookup: lookup({ madeFor: '1.0.11', newerNeeded: true }) }))!
  assert.equal(v.line, 'online-fix.me has an online fix for this game (11 MB).')
  assert.equal(v.get, 'primary')
  assert.ok(v.fromFile)
  assert.match(v.version!, /newer than yours \(1\.0\.9\)\. Update the game first/)
  assert.equal(versionLine(lookup({ madeFor: '1.0.9' }), '1.0.9'), 'Made for version 1.0.9. It may need that same version.')
})

test('no fix, no answer and a sign-in wall each say what to do', () => {
  const none = onlineFixView(state({ lookup: lookup({ status: 'none', file: null }) }))!
  assert.equal(none.line, 'online-fix.me has no online fix for this game.')
  assert.equal(none.get, null)
  const down = onlineFixView(state({ lookup: lookup({ status: 'unreachable', file: null }) }))!
  assert.ok(down.retry && down.fromFile && /did not answer/.test(down.line))
  const wall = onlineFixView(state({ lookup: lookup({ status: 'signin', file: null }) }))!
  assert.ok(wall.openPage && /sign in/.test(wall.line))
})

test('a fix Kryoto put in can be removed; one the release carries is only put back', () => {
  const on = onlineFixView(state({ applied: true, lookup: lookup() }))!
  assert.ok(on.remove)
  assert.equal(on.get, null)
  const carried = onlineFixView(state({ carried: true, lookup: lookup() }))!
  assert.equal(carried.get, 'again')
  assert.ok(!carried.remove)
  assert.match(carried.line, /already has an online fix/)
})

test('player words only: no em dashes, no staff tool names', () => {
  for (const s of [state({ lookup: lookup() }), state({ applied: true }), state({ carried: true }), state({ lookup: lookup({ status: 'unreachable' }) }), state({ lookup: lookup({ status: 'signin' }) })]) {
    const v = onlineFixView(s)!
    assert.ok(!/—|Steamless|Forge|fixpack/.test(v.line + (v.version ?? '')), v.line)
  }
})
