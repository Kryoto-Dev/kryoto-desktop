/**
 * What the public room's list does with one read of kryo.to
 * (`GET /api/chat/room?after=&since=`): new messages are added, messages that
 * were edited or reacted to since the last read are replaced by how they are
 * now, and removed ones go. Pure, so it is tested (room-feed.test.ts) and the
 * pane (RoomPane.tsx) only draws.
 */

export type RoomReaction = { emoji: string; count: number; mine: boolean; people: { name: string; username: string | null }[] }

export type RoomMessage = {
  id: string
  body: string
  /** The message this answers; `gone` when it was removed or cannot be shown. */
  replyTo?: { id: string; gone: boolean; name: string | null; username: string | null; excerpt: string } | null
  createdAt: string
  /** When its author last changed the words. */
  editedAt?: string | null
  /** Written in #kryoto-chat on the Kryoto Discord, by this account's linked Discord. */
  origin?: 'discord' | null
  reactions?: RoomReaction[]
  user: { id: string; username: string; displayName: string | null; avatarUrl: string | null; supporter: boolean }
}

export type RoomRead = {
  messages: RoomMessage[]
  /** Older messages edited or reacted to since `since`, as they are now. Absent from a kryo.to that does not send it yet. */
  changed?: RoomMessage[]
  /** Ids removed since `since`. */
  removed?: string[]
  /** kryo.to's clock for this read: the next read's `since`. */
  now?: string
  canModerate: boolean
}

/** The room keeps this many messages on screen. */
export const ROOM_KEPT = 300
/** A message can be changed for a day after it was sent (kryo.to enforces it; this only hides the button). */
export const ROOM_EDIT_WINDOW_MS = 24 * 60 * 60 * 1000

export function canEditRoomMessage(m: Pick<RoomMessage, 'createdAt'>, now: number): boolean {
  const sent = Date.parse(m.createdAt)
  return Number.isFinite(sent) && now - sent < ROOM_EDIT_WINDOW_MS
}

export function applyRoomRead(list: RoomMessage[], read: RoomRead): RoomMessage[] {
  const removed = new Set(read.removed ?? [])
  const changed = new Map((read.changed ?? []).map((m) => [m.id, m]))
  const fresh = new Map(read.messages.map((m) => [m.id, m]))
  // A change only replaces a message that is on screen: one from before the window is not brought back.
  const kept = list.filter((m) => !removed.has(m.id)).map((m) => fresh.get(m.id) ?? changed.get(m.id) ?? m)
  const seen = new Set(kept.map((m) => m.id))
  return [...kept, ...read.messages.filter((m) => !seen.has(m.id) && !removed.has(m.id))].slice(-ROOM_KEPT)
}
