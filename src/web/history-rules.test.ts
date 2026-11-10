// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  ASK_AGAIN_MIN_MS,
  ASK_AGAIN_MS,
  askAgainDue,
  EXPIRE_MS,
  incomingView,
  keptOffers,
  onChunk,
  onRepeatRequest,
  QUIET_MS,
  STALL_MS,
  withOffer,
  type SavedOffer,
  type Want,
} from './history-rules.ts'

const T0 = 1_800_000_000_000
const waiting: Want = { transferId: 'aa', state: 'waiting', from: null, received: 0, total: 0, announced: true, askedAtMs: T0 }

test('a waiting device asks again by itself until someone answers (KB-0254)', () => {
  // Just asked: not yet. Then on the timer, and sooner when another device is in use.
  assert.equal(askAgainDue(waiting, T0 + 1000, T0, false), false)
  assert.equal(askAgainDue(waiting, T0 + ASK_AGAIN_MS, T0, false), true)
  assert.equal(askAgainDue(waiting, T0 + ASK_AGAIN_MIN_MS, T0, true), true)
  assert.equal(askAgainDue(waiting, T0 + ASK_AGAIN_MIN_MS - 1, T0, true), false)
  // Done, nothing asked, or given up: never.
  assert.equal(askAgainDue(null, T0 + ASK_AGAIN_MS, T0, true), false)
  assert.equal(askAgainDue({ ...waiting, state: 'done' }, T0 + ASK_AGAIN_MS, T0, true), false)
  assert.equal(askAgainDue(waiting, T0 + EXPIRE_MS, T0 + EXPIRE_MS - ASK_AGAIN_MS, true), false)
  // Receiving: only once it stalled.
  const receiving: Want = { ...waiting, state: 'receiving', fromDevice: '7', lastAtMs: T0 + 5000 }
  assert.equal(askAgainDue(receiving, T0 + 5000 + STALL_MS - 1, T0, false), false)
  assert.equal(askAgainDue(receiving, T0 + 5000 + STALL_MS, T0, false), true)
})

test('the waiting line turns useful after a while and says when it gave up', () => {
  assert.deepEqual(
    [incomingView(waiting, T0 + 1000).quiet, incomingView(waiting, T0 + QUIET_MS).quiet, incomingView(waiting, T0 + QUIET_MS).expired],
    [false, true, false],
  )
  assert.equal(incomingView(waiting, T0 + EXPIRE_MS).expired, true)
  // A request from 0.3.1 has no time: it counts from now, not from 1970.
  const old = incomingView({ ...waiting, askedAtMs: undefined }, T0)
  assert.deepEqual([old.askedAtMs, old.quiet, old.expired], [T0, false, false])
  // A done one is neither quiet nor expired, however old.
  const done = incomingView({ ...waiting, state: 'done' }, T0 + EXPIRE_MS * 2)
  assert.deepEqual([done.quiet, done.expired], [false, false])
})

test('two devices saying yes make one transfer, nothing twice', () => {
  // The first to send is the one it comes from.
  const a = onChunk(waiting, 'aa', 'A', 'me', 0, 3, T0)!
  assert.deepEqual([a.use, a.done, a.ackNext, a.want.fromDevice, a.want.state, a.want.received], ['progress', false, 1, 'A', 'receiving', 1])
  // A second one is told it can stop (ack = its total); its rows are merged by id, progress unchanged.
  const b = onChunk(a.want, 'aa', 'B', 'me', 0, 2, T0 + 1000)!
  assert.deepEqual([b.use, b.done, b.ackNext, b.want], ['extra', false, 2, a.want])
  // Even its last part does not finish the transfer: A does.
  assert.equal(onChunk(a.want, 'aa', 'B', 'me', 1, 2, T0 + 1000)!.done, false)
  const a2 = onChunk(a.want, 'aa', 'A', 'me', 2, 3, T0 + 2000)!
  assert.deepEqual([a2.use, a2.done, a2.want.state, a2.want.received], ['progress', true, 'done', 3])
  // After done, nothing more counts; nor other requests, nor our own echo.
  assert.equal(onChunk(a2.want, 'aa', 'B', 'me', 1, 2, T0 + 3000), null)
  assert.equal(onChunk(waiting, 'bb', 'A', 'me', 0, 1, T0), null)
  assert.equal(onChunk(waiting, 'aa', 'me', 'me', 0, 1, T0), null)
  assert.equal(onChunk(waiting, 'aa', 'A', 'me', 3, 3, T0), null)
  assert.equal(onChunk(waiting, 'aa', 'A', 'me', 0, 0, T0), null)
})

test('another device takes over when the first one stalls', () => {
  const a = onChunk(waiting, 'aa', 'A', 'me', 0, 3, T0)!.want
  const b = onChunk(a, 'aa', 'B', 'me', 0, 2, T0 + STALL_MS)!
  assert.deepEqual([b.use, b.want.fromDevice, b.want.from, b.want.received, b.want.total], ['progress', 'B', null, 1, 2])
})

const offer = (id: string, seenAtMs: number, later = false): SavedOffer => ({ transferId: id, deviceId: '9', deviceName: 'Kryoto Mobile', createdAtMs: seenAtMs, seenAtMs, later })

test('an old device keeps a request until it is answered, done or expired', () => {
  const saved = [offer('x', T0), offer('y', T0 - EXPIRE_MS), offer('z', T0, true), offer('w', T0)]
  const { keep, expired } = keptOffers(saved, ['w'], T0 + 1000)
  // Closed (done elsewhere) goes; expired goes to the closed list; "Not now" stays kept, so it is not asked again.
  assert.deepEqual(keep.map((s) => [s.transferId, s.later]), [['x', false], ['z', true]])
  assert.deepEqual(expired, ['y'])
  // Saving again replaces, and the list stays short.
  const list = withOffer(withOffer([offer('x', T0)], { ...offer('x', T0), later: true }), offer('v', T0))
  assert.deepEqual(list.map((s) => [s.transferId, s.later]), [['x', true], ['v', false]])
  let many: SavedOffer[] = []
  for (let i = 0; i < 30; i++) many = withOffer(many, offer(String(i), T0))
  assert.equal(many.length, 20)
  assert.equal(many[19]!.transferId, '29')
})

test('a request that comes again picks up where the transfer stood', () => {
  const o = { state: 'sending' as const, hasChunks: true, pumping: false, next: 1, total: 3 }
  assert.equal(onRepeatRequest({ ...o, state: 'asking' }), 'none')
  assert.equal(onRepeatRequest({ ...o, state: 'done' }), 'none')
  assert.equal(onRepeatRequest({ ...o, hasChunks: false }), 'start')
  assert.equal(onRepeatRequest({ ...o, pumping: true }), 'none')
  assert.equal(onRepeatRequest({ ...o, state: 'waiting' }), 'resume')
  // Told to stop (another device was sending) but still asked: that one stalled, so from the start.
  assert.equal(onRepeatRequest({ ...o, next: 3 }), 'restart')
})
