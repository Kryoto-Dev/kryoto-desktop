// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { moveIn, orderList, orderSources } from './download-order.ts'

const ours = { kind: 'ours' }
const pixeldrain = { kind: 'mirror', host: 'Pixeldrain' }
const gofile = { kind: 'mirror', host: 'Gofile' }
const viking = { kind: 'mirror', host: 'VikingFile' }

test("no order is kryo.to's: the sources stay as they came", () => {
  const sources = [ours, pixeldrain, gofile, viking]
  assert.equal(orderSources(sources, []), sources)
})

test("the player's first pick the release has comes first", () => {
  assert.deepEqual(orderSources([ours, pixeldrain, gofile], ['Gofile', 'kryoto', 'Pixeldrain']), [gofile, ours, pixeldrain])
  // Not on this build: the next pick leads.
  assert.deepEqual(orderSources([ours, pixeldrain], ['Gofile', 'Pixeldrain', 'kryoto']), [pixeldrain, ours])
  // Names match whatever their case.
  assert.deepEqual(orderSources([ours, gofile], ['gofile']), [gofile, ours])
})

test('hosts the order does not name follow, in the order they came', () => {
  assert.deepEqual(orderSources([ours, pixeldrain, gofile, viking], ['VikingFile']), [viking, ours, pixeldrain, gofile])
})

test('an archived build has no copy of ours, and its mirrors still follow the order', () => {
  assert.deepEqual(orderSources([pixeldrain, gofile], ['kryoto', 'Gofile']), [gofile, pixeldrain])
})

test('Settings lists the saved order, then every other host, our copy next when unplaced', () => {
  assert.deepEqual(orderList([], ['Pixeldrain', 'Gofile']), ['kryoto', 'Pixeldrain', 'Gofile'])
  assert.deepEqual(orderList(['Gofile', 'kryoto'], ['Pixeldrain', 'Gofile']), ['Gofile', 'kryoto', 'Pixeldrain'])
  assert.deepEqual(orderList(['Gofile'], ['Pixeldrain', 'Gofile']), ['Gofile', 'kryoto', 'Pixeldrain'])
})

test('moving stays inside the list', () => {
  assert.deepEqual(moveIn(['a', 'b', 'c'], 1, -1), ['b', 'a', 'c'])
  assert.deepEqual(moveIn(['a', 'b', 'c'], 1, 1), ['a', 'c', 'b'])
  const list = ['a', 'b']
  assert.equal(moveIn(list, 0, -1), list)
  assert.equal(moveIn(list, 1, 1), list)
})
