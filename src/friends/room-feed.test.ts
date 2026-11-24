import assert from 'node:assert/strict'
import { test } from 'node:test'
import { applyRoomRead, canEditRoomMessage, ROOM_EDIT_WINDOW_MS, type RoomMessage } from './room-feed.ts'

const msg = (id: string, body = `m${id}`, over: Partial<RoomMessage> = {}): RoomMessage => ({
  id,
  body,
  createdAt: '2026-11-24T10:00:00.000Z',
  user: { id: '7', username: 'mira', displayName: 'Mira', avatarUrl: null, supporter: false },
  ...over,
})

test('new messages are added once, in order', () => {
  const list = applyRoomRead([msg('1')], { messages: [msg('1'), msg('2')], canModerate: false })
  assert.deepEqual(list.map((m) => m.id), ['1', '2'])
})

test('an edit, from here or from Discord, replaces the message where it stands and marks it', () => {
  const list = applyRoomRead([msg('1'), msg('2'), msg('3')], {
    messages: [],
    changed: [msg('2', 'now this', { editedAt: '2026-11-24T10:05:00.000Z' })],
    canModerate: false,
  })
  assert.deepEqual(list.map((m) => [m.id, m.body, Boolean(m.editedAt)]), [['1', 'm1', false], ['2', 'now this', true], ['3', 'm3', false]])
})

test('reactions on an older message arrive the same way', () => {
  const reactions = [{ emoji: '\u{1F525}', count: 2, mine: false, people: [{ name: 'Ash', username: 'ash' }, { name: 'Anonymous', username: null }] }]
  const list = applyRoomRead([msg('1')], { messages: [], changed: [msg('1', 'm1', { reactions })], canModerate: false })
  assert.deepEqual(list[0]!.reactions, reactions)
})

test('a removed message goes, whoever removed it and wherever', () => {
  const list = applyRoomRead([msg('1'), msg('2')], { messages: [msg('3')], removed: ['1'], canModerate: false })
  assert.deepEqual(list.map((m) => m.id), ['2', '3'])
  // Removed in the same read it arrived in: never shown.
  assert.deepEqual(applyRoomRead([], { messages: [msg('4')], removed: ['4'], canModerate: false }), [])
})

test('a change to a message that is not on screen does not bring it back', () => {
  const list = applyRoomRead([msg('5')], { messages: [], changed: [msg('1', 'old one, edited')], canModerate: false })
  assert.deepEqual(list.map((m) => m.id), ['5'])
})

test('a kryo.to that sends no changes yet still works', () => {
  assert.deepEqual(applyRoomRead([msg('1')], { messages: [msg('2')], canModerate: true }).map((m) => m.id), ['1', '2'])
})

test('your message can be changed for a day', () => {
  const sent = Date.parse('2026-11-24T10:00:00.000Z')
  assert.equal(canEditRoomMessage(msg('1'), sent + 60_000), true)
  assert.equal(canEditRoomMessage(msg('1'), sent + ROOM_EDIT_WINDOW_MS + 1), false)
  assert.equal(canEditRoomMessage({ createdAt: 'nonsense' }, sent), false)
})
