/**
 * Chat history from one of an account's devices to another (KB-0254): the
 * rules both ends follow, apart from the engine so they are tested on their
 * own (history-rules.test.ts). Native chat follows the same rules
 * (src-tauri/src/chat/history.rs).
 *
 * Nothing of a request may get lost: the new device asks again, live, while
 * it waits (and sooner when another of its devices says it is in use), and an
 * older device keeps a request it was asked until it is answered, done or
 * expired, so closing it before answering does not drop the question.
 */

/** A new device counts as just set up for this long ("Send" is the default). */
export const FRESH_MS = 10 * 60_000
/** While it waits, the new device asks its online devices again this often (live, never stored). */
export const ASK_AGAIN_MS = 45_000
/** Sooner when one of them says it is in use, but never more often than this. */
export const ASK_AGAIN_MIN_MS = 10_000
/** A transfer that brought nothing for this long has stalled: ask again, and another device may take over. */
export const STALL_MS = 60_000
/** Nobody answered for this long: the waiting line says what to do. */
export const QUIET_MS = 2 * 60_000
/** A request is given up after this long, on both ends. */
export const EXPIRE_MS = 7 * 24 * 3600_000

/** On the new device: the history it asked for, as kept in its store. */
export type Want = {
  transferId: string
  state: 'waiting' | 'receiving' | 'done'
  from: string | null
  received: number
  total: number
  /** The stored request went out (later ones are live only). */
  announced?: boolean
  /** When it first asked (requests from 0.3.1 have none: counted from the first start that sees them). */
  askedAtMs?: number
  /** The device it takes the history from: the first that sends. */
  fromDevice?: string | null
  /** When the last part from that device came. */
  lastAtMs?: number
}

/** What the new device shows (lib/chat.ts `HistoryIncoming`). */
export type IncomingView = {
  transferId: string
  state: Want['state']
  from: string | null
  received: number
  total: number
  askedAtMs: number
  /** Waiting and nobody answered for a while, or receiving and it stalled. */
  quiet: boolean
  /** Waited so long it no longer asks. */
  expired: boolean
}

export function expired(w: Want, now: number): boolean {
  return w.state !== 'done' && now - (w.askedAtMs ?? now) >= EXPIRE_MS
}

export function incomingView(w: Want, now: number): IncomingView {
  const askedAtMs = w.askedAtMs ?? now
  const quiet =
    w.state === 'waiting' ? now - askedAtMs >= QUIET_MS : w.state === 'receiving' ? now - (w.lastAtMs ?? askedAtMs) >= STALL_MS : false
  return { transferId: w.transferId, state: w.state, from: w.from, received: w.received, total: w.total, askedAtMs, quiet, expired: expired(w, now) }
}

/**
 * Whether the new device asks its online devices again now: while it waits,
 * or when the transfer stalled; every `ASK_AGAIN_MS`, or after
 * `ASK_AGAIN_MIN_MS` when another of its devices was `nudged` into use.
 */
export function askAgainDue(w: Want | null, now: number, lastAskMs: number, nudged: boolean): boolean {
  if (!w || w.state === 'done' || expired(w, now)) return false
  if (w.state === 'receiving' && now - (w.lastAtMs ?? 0) < STALL_MS) return false
  const gap = now - lastAskMs
  return gap >= ASK_AGAIN_MS || (nudged && gap >= ASK_AGAIN_MIN_MS)
}

/**
 * A part of the history from `sender`. `progress`: it counts (the device the
 * history comes from, or the first one, or one taking over from a stalled
 * one). `extra`: a second device said yes too; its rows are merged (by
 * message id, never shown twice) and it is told it can stop (`ackNext` =
 * its total). Null: not for this request, or already done.
 */
export function onChunk(
  w: Want | null,
  transferId: string,
  sender: string,
  me: string,
  index: number,
  total: number,
  now: number,
): { use: 'progress' | 'extra'; done: boolean; ackNext: number; want: Want } | null {
  if (!w || w.transferId !== transferId || w.state === 'done' || sender === me) return null
  if (!Number.isInteger(index) || !Number.isInteger(total) || total < 1 || index < 0 || index >= total) return null
  const lock = w.fromDevice ?? null
  if (lock !== null && lock !== sender && now - (w.lastAtMs ?? 0) < STALL_MS) {
    return { use: 'extra', done: false, ackNext: total, want: w }
  }
  const done = index + 1 >= total
  const want: Want = {
    ...w,
    state: done ? 'done' : 'receiving',
    from: lock === sender ? w.from : null,
    fromDevice: sender,
    received: index + 1,
    total,
    lastAtMs: now,
  }
  return { use: 'progress', done, ackNext: index + 1, want }
}

/** On an older device: a request it was asked, kept until it is answered, done or expired. */
export type SavedOffer = {
  transferId: string
  deviceId: string
  deviceName: string
  createdAtMs: number
  /** When this device first saw it. */
  seenAtMs: number
  /** "Not now": not asked again for this request (a new request from that device is). */
  later: boolean
}

/** The kept requests still worth asking, and the expired ones (which go on the closed list). */
export function keptOffers(saved: SavedOffer[], closed: string[], now: number): { keep: SavedOffer[]; expired: string[] } {
  const keep: SavedOffer[] = []
  const gone: string[] = []
  for (const s of saved) {
    if (closed.includes(s.transferId)) continue
    if (now - s.seenAtMs >= EXPIRE_MS) gone.push(s.transferId)
    else keep.push(s)
  }
  return { keep, expired: gone }
}

/** Add or replace one kept request (by transfer id), at most 20 kept. */
export function withOffer(saved: SavedOffer[], o: SavedOffer): SavedOffer[] {
  return [...saved.filter((s) => s.transferId !== o.transferId), o].slice(-20)
}

/**
 * A request came again (live) for an offer this device already has.
 * `start`: agreed, but nothing cut yet (a restart lost it); `resume`: carry
 * on from the last acknowledged part; `restart`: it was told to stop (another
 * device sent) or ran out, yet the new device still asks: from the start
 * (merging by id makes repeats harmless); `none`: nothing to do (still asking
 * its player, said Not now, busy sending, or done).
 */
export function onRepeatRequest(o: { state: 'asking' | 'sending' | 'waiting' | 'done'; hasChunks: boolean; pumping: boolean; next: number; total: number }): 'start' | 'resume' | 'restart' | 'none' {
  if (o.state !== 'sending' && o.state !== 'waiting') return 'none'
  if (!o.hasChunks) return 'start'
  if (o.pumping) return 'none'
  return o.next >= o.total ? 'restart' : 'resume'
}
