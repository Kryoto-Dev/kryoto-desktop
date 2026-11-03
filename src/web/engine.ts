/**
 * The web chat engine: what src-tauri/src/chat/{engine,messaging,identity}.rs
 * do on the desktop, in the browser. The cryptography and the wire format
 * are km-core's, compiled to WebAssembly (`Kryo`); this file connects to the
 * gateway, keeps the encrypted store up to date, and speaks to the shared
 * chat UI through the same events the desktop app emits.
 *
 * Same rules as the desktop:
 * - a delivery is decrypted, stored, then the device state saved, then
 *   acknowledged (a stop in between means a redelivery the core recognises);
 * - every stored send covers the other person's devices and our own;
 * - groups fan out per member device and name the group to the gateway;
 * - receipts and typing never go to someone whose request is still pending.
 */

import init, { Kryo } from './pkg/km_wasm'
import { api, GATEWAY_HTTP, GATEWAY_WS } from './api'
import { Store, type MessageRow } from './store'
import { EMBEDDED, toSite } from './embed'

export type ChatStatus =
  | { state: 'off' }
  | { state: 'connecting' }
  | { state: 'online'; userId: string; deviceId: string }
  | { state: 'offline'; retryInSecs: number }
  | { state: 'signInNeeded' }
  | { state: 'anonymousMode' }
  | { state: 'disabled' }
  | { state: 'needsLink' }
  | { state: 'deviceRemoved' }
  | { state: 'unavailable'; reason: string }

export type Person = { id: string; name: string; supporter: boolean; muted: boolean; pending?: boolean }
export type Settings = { readReceipts: boolean; typing: boolean; notificationContent: 'full' | 'name' | 'none'; gifsAuto: boolean }
type Frame = Record<string, any> & { type: string; requestId: number }
type Target = { dm: string } | { group: string }

const DEFAULT_SETTINGS: Settings = { readReceipts: true, typing: true, notificationContent: 'name', gifsAuto: true }
const LOW_WATER = 30
const TARGET = 100
const REQUEST_TIMEOUT = 20_000
const EDIT_WINDOW_MS = 24 * 3600 * 1000
const INVITE_TTL_MS = 15 * 60 * 1000

export class SendError extends Error {}

const hexOf = (n: bigint) => n.toString(16).padStart(16, '0')
export function dmConversationId(a: string, b: string): string {
  const [lo, hi] = BigInt(a) <= BigInt(b) ? [BigInt(a), BigInt(b)] : [BigInt(b), BigInt(a)]
  return `646d3a${hexOf(lo)}${hexOf(hi)}`
}
export function groupConversationId(g: string): string {
  return `6772703a${hexOf(BigInt(g))}`
}
function targetOf(conv: string, me: string): Target | null {
  if (conv.startsWith('6772703a') && conv.length === 24) return { group: BigInt(`0x${conv.slice(8)}`).toString() }
  if (conv.startsWith('646d3a') && conv.length === 38) {
    const lo = BigInt(`0x${conv.slice(6, 22)}`).toString()
    const hi = BigInt(`0x${conv.slice(22)}`).toString()
    if (lo === me) return { dm: hi }
    if (hi === me) return { dm: lo }
  }
  return null
}
export function parseTarget(s: string): Target {
  return s.startsWith('g:') ? { group: s.slice(2) } : { dm: s }
}
const convOf = (t: Target, me: string) => ('group' in t ? groupConversationId(t.group) : dmConversationId(me, t.dm))

function b64ToBytes(s: string): Uint8Array {
  const bin = atob(s)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

export class WebChat {
  status: ChatStatus = { state: 'connecting' }
  people = new Map<string, Person>()
  meSupporter = false
  private ws: WebSocket | null = null
  private nextId = 1
  private pending = new Map<number, (f: Frame) => void>()
  private groups = new Map<string, { userId: string; role: string }[]>()
  private queue: Promise<void> = Promise.resolve()
  private stopped = false
  private backoff = 1
  /** The gateway's VAPID key (hex), empty when it sends no pushes. */
  private pushKey = ''
  /** Calls that have rung here already: one notice each. */
  private rung = new Set<string>()
  /**
   * Until when another of this account's devices is in use (it sent
   * `active`): that one shows the notifications, this one keeps quiet.
   */
  private activeElsewhereUntil = 0

  private constructor(
    private kryo: Kryo,
    private store: Store,
    readonly token: string,
    readonly userId: string,
    private emit: (event: string, payload: unknown) => void,
  ) {}

  static async open(store: Store, token: string, userId: string, emit: (e: string, p: unknown) => void): Promise<WebChat> {
    await init()
    const saved = await store.get<string>('device')
    const kryo = saved ? Kryo.restore(saved) : Kryo.create(userId)
    if (kryo.userId() !== userId) throw new Error('This browser holds chat for another account. Remove it first.')
    const chat = new WebChat(kryo, store, token, userId, emit)
    await chat.persist()
    return chat
  }

  private async persist() {
    await this.store.set('device', this.kryo.state())
  }

  private setStatus(s: ChatStatus) {
    this.status = s
    this.emit('chat-status', s)
  }

  // ---- connection -------------------------------------------------------------

  run() {
    this.stopped = false
    void this.loop()
  }

  stop() {
    this.stopped = true
    this.ws?.close()
  }

  private async loop() {
    while (!this.stopped) {
      this.setStatus({ state: 'connecting' })
      const started = Date.now()
      const outcome = await this.session().catch((e: unknown) => ({ retry: String(e) }))
      if (this.stopped) return
      if ('stop' in outcome) {
        this.setStatus(outcome.stop)
        return
      }
      if (Date.now() - started > 60_000) this.backoff = 1
      this.setStatus({ state: 'offline', retryInSecs: this.backoff })
      await new Promise((r) => setTimeout(r, this.backoff * (875 + Math.random() * 250)))
      this.backoff = Math.min(this.backoff * 2, 60)
    }
  }

  private refusal(code: string, message: string): { stop: ChatStatus } | { retry: string } {
    switch (code) {
      case 'unauthorized':
        return { stop: { state: 'signInNeeded' } }
      case 'device_unknown':
        return { stop: { state: 'deviceRemoved' } }
      case 'anonymous_mode':
        return { stop: { state: 'anonymousMode' } }
      case 'chat_disabled':
        return { stop: { state: 'disabled' } }
      case 'unsupported_version':
      case 'conflict':
        return { stop: { state: 'unavailable', reason: message } }
      default:
        return { retry: code }
    }
  }

  /** One connection, until it ends. */
  private session(): Promise<{ stop: ChatStatus } | { retry: string }> {
    return new Promise((resolve) => {
      const ws = new WebSocket(GATEWAY_WS)
      ws.binaryType = 'arraybuffer'
      this.ws = ws
      let phase: 'hello' | 'register' | 'ready' | 'live' = 'hello'
      let done = false
      const finish = (o: { stop: ChatStatus } | { retry: string }) => {
        if (done) return
        done = true
        for (const [, r] of this.pending) r({ type: 'error', requestId: 0, code: 'offline', message: 'Not connected.' })
        this.pending.clear()
        try {
          ws.close()
        } catch {
          // already closed
        }
        resolve(o)
      }
      ws.onopen = () => ws.send(this.kryo.hello(this.token))
      ws.onclose = () => finish({ retry: 'connection closed' })
      ws.onerror = () => finish({ retry: 'connection error' })
      ws.onmessage = (ev) => {
        let f: Frame
        try {
          f = JSON.parse(Kryo.decode(new Uint8Array(ev.data as ArrayBuffer))) as Frame
        } catch {
          return
        }
        if (f.type === 'error' && phase !== 'live') {
          finish(this.refusal(f.code, f.message))
          return
        }
        if (phase === 'hello' && f.type === 'challenge') {
          const nonce = b64ToBytes(f.nonce)
          if (this.kryo.deviceId()) {
            ws.send(this.kryo.proof(nonce))
            phase = 'ready'
          } else {
            ws.send(this.kryo.register(nonce, 'Web browser'))
            phase = 'register'
          }
          return
        }
        if (phase === 'register' && f.type === 'registered') {
          this.kryo.setDeviceId(f.deviceId)
          void this.persist()
          phase = 'ready'
          return
        }
        if (phase === 'ready' && f.type === 'ready') {
          phase = 'live'
          this.pushKey = typeof f.pushKey === 'string' ? f.pushKey : ''
          void this.settle(f)
            .then(() => {
              this.backoff = 1
              this.setStatus({ state: 'online', userId: f.userId, deviceId: f.deviceId })
              void this.resendPending()
              void this.pushResync()
            })
            .catch((e: unknown) => finish(e && typeof e === 'object' && 'stop' in e ? (e as { stop: ChatStatus }) : { retry: String(e) }))
          return
        }
        if (phase !== 'live') return
        if (f.requestId) {
          this.pending.get(f.requestId)?.(f)
          this.pending.delete(f.requestId)
          return
        }
        if (f.type === 'deliver') {
          this.queue = this.queue.then(() => this.handleDelivery(f)).catch(() => {})
          return
        }
        if (f.type === 'event') {
          if (f.event === 'masterKeyChanged' && f.userId === this.userId) finish({ retry: 'own master key changed' })
          else if (f.event === 'groupChanged') void this.onGroupChanged(f.groupId)
          return
        }
        if (f.type === 'error') finish(this.refusal(f.code, f.message))
      }
    })
  }

  private async settle(ready: Frame) {
    const ours = this.kryo.masterPublic()
    if (!ready.masterKey) {
      await this.ask('publishMasterKey')
    } else if (ready.masterKey !== ours) {
      throw { stop: { state: 'needsLink' } as ChatStatus }
    }
    if (!ready.certified || !ready.masterKey) await this.ask('certify')
    if (ready.oneTimeKeys < LOW_WATER || !ready.hasFallbackKey) {
      const rid = this.nextId++
      const frame = this.kryo.request(rid, 'keysUpload', JSON.stringify({ count: TARGET - ready.oneTimeKeys, fallback: !ready.hasFallbackKey }))
      // Saved before upload: the server must never hold a key whose secret we could lose.
      await this.persist()
      await this.raw(rid, frame)
    }
  }

  private raw(rid: number, frame: Uint8Array): Promise<Frame> {
    return new Promise((resolve, reject) => {
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        reject(new SendError('You are offline. It will be sent when you are back.'))
        return
      }
      const timer = setTimeout(() => {
        this.pending.delete(rid)
        reject(new SendError('The chat server did not answer.'))
      }, REQUEST_TIMEOUT)
      this.pending.set(rid, (f) => {
        clearTimeout(timer)
        if (f.type === 'error') reject(new SendError(f.message || f.code))
        else resolve(f)
      })
      this.ws.send(frame)
    })
  }

  private ask(kind: string, args: Record<string, unknown> = {}): Promise<Frame> {
    const rid = this.nextId++
    return this.raw(rid, this.kryo.request(rid, kind, JSON.stringify(args)))
  }

  get online() {
    return this.status.state === 'online'
  }

  // ---- people ---------------------------------------------------------------

  private supporter(id: string) {
    return this.people.get(id)?.supporter ?? false
  }
  private awaiting(id: string) {
    return this.people.get(id)?.pending ?? false
  }

  async settings(): Promise<Settings> {
    return { ...DEFAULT_SETTINGS, ...((await this.store.get<Settings>('settings')) ?? {}) }
  }
  async setSettings(s: Settings) {
    await this.store.set('settings', s)
  }

  /** The supporter rules, as the reader sees a stored message. */
  shown(row: MessageRow): MessageRow {
    if (row.kind === 'file') {
      try {
        const f = JSON.parse(row.body) as Record<string, unknown>
        return { ...row, body: JSON.stringify({ name: f.name, mime: f.mime, size: f.size, width: f.width, height: f.height }) }
      } catch {
        return row
      }
    }
    if (row.outgoing || row.deleted || row.kind === 'invite') return row
    const body =
      row.kind === 'gif'
        ? { type: 'gif', ...(JSON.parse(row.body) as object) }
        : { type: 'text', text: row.body, replyTo: '' }
    const content = JSON.stringify({ msgId: row.msgId, conversationId: row.conversationId, sentAt: row.sentAt, body })
    const r = JSON.parse(Kryo.receiveRules(content, this.supporter(row.senderUser))) as { shown: string; text?: string }
    if (r.shown === 'truncated') return { ...row, body: r.text ?? row.body }
    if (r.shown === 'gifAsText') return { ...row, kind: 'text', body: r.text ?? 'GIF' }
    return row
  }

  // ---- devices and delivery ------------------------------------------------------

  /** A user's checked devices: fetched unless known. Returns how many. */
  private async devicesOf(user: string, refresh: boolean): Promise<number> {
    const known = this.kryo.knownDevices(user)
    if (!refresh && known > 0) return known
    const f = await this.ask('devicesQuery', { userIds: [user] })
    const r = JSON.parse(this.kryo.checkDevices(JSON.stringify(f), user)) as { status: string; count: number }
    if (r.status === 'identityChanged') {
      this.emit('chat-identity-changed', { userId: user })
      throw new SendError('Their security key changed. Check it before sending.')
    }
    await this.persist()
    return r.count
  }

  private async deliver(users: string[], content: object, ephemeral: boolean, group: string | null): Promise<void> {
    if (!this.online) throw new SendError('You are offline. It will be sent when you are back.')
    for (let attempt = 0; attempt < 2; attempt++) {
      const reach: string[] = []
      for (const u of users) {
        let n: number
        try {
          n = await this.devicesOf(u, attempt > 0)
        } catch (e) {
          if (group) continue
          throw e
        }
        if (n === 0 && u !== this.userId) {
          if (group) continue
          throw new SendError('They have not turned on chat yet.')
        }
        if (this.kryo.needsClaim(u)) {
          const b = await this.ask('keysClaim', { userId: u }).catch(() => null)
          if (b) this.kryo.addClaims(JSON.stringify(b))
        }
        reach.push(u)
      }
      const rid = this.nextId++
      let frame: Uint8Array
      try {
        frame = this.kryo.sendFrame(rid, JSON.stringify(content), JSON.stringify(reach), ephemeral, group ?? '')
      } catch {
        if (group || users.every((u) => u === this.userId)) return
        throw new SendError('They have not turned on chat yet.')
      }
      await this.persist()
      const ack = await this.raw(rid, frame)
      if (ack.status === 'accepted') return
      if (ack.status === 'deviceMismatch') continue
      if (ack.status === 'forbidden') throw new SendError('You cannot message this person.')
      if (ack.status === 'rateLimited') throw new SendError('Slow down a little.')
      throw new SendError('Unexpected answer.')
    }
    throw new SendError('Their devices kept changing. Try again.')
  }

  private async recipients(t: Target): Promise<{ users: string[]; group: string | null }> {
    if ('dm' in t) return { users: [t.dm, this.userId], group: null }
    const members = await this.groupMembers(t.group, false)
    if (!members?.some((m) => m.userId === this.userId)) throw new SendError('You are not in that group.')
    return { users: members.map((m) => m.userId), group: t.group }
  }

  private content(t: Target, body: object, msgId = Kryo.newMessageId(), sentAt = Date.now()) {
    return { msgId, conversationId: convOf(t, this.userId), sentAt, body }
  }

  private async sendNew(t: Target, body: Record<string, unknown>, kind: MessageRow['kind'], stored: string, replyTo: string | null): Promise<MessageRow> {
    const c = this.content(t, body)
    try {
      Kryo.validate(JSON.stringify(c), this.meSupporter)
    } catch {
      const limit = this.meSupporter ? 8000 : 2000
      throw new SendError(body.type === 'text' && String(body.text).length > limit ? `Messages can be ${limit} characters long${this.meSupporter ? '' : ' (8,000 for supporters)'}.` : 'That cannot be sent.')
    }
    const { users, group } = await this.recipients(t)
    const row: MessageRow = {
      msgId: c.msgId,
      conversationId: c.conversationId,
      peerUserId: 'dm' in t ? t.dm : null,
      senderUser: this.userId,
      senderDevice: this.kryo.deviceId() ?? '0',
      outgoing: true,
      sentAt: c.sentAt,
      receivedAt: c.sentAt,
      kind,
      body: stored,
      replyTo,
      editedAt: null,
      deleted: false,
      status: 'sending',
      reactions: [],
    }
    await this.store.insert(row)
    this.emit('chat-message', this.shown(row))
    let status: MessageRow['status'] | null = 'sent'
    let failure: unknown = null
    try {
      await this.deliver(users, c, false, group)
    } catch (e) {
      failure = e
      status = this.online ? 'failed' : null
    }
    const after = status ? ((await this.store.advance(row.msgId, status)) ?? row) : row
    this.emit('chat-updated', this.shown(after))
    if (failure && this.online) throw failure
    return this.shown(after)
  }

  sendText(t: Target, text: string, replyTo: string | null) {
    return this.sendNew(t, { type: 'text', text, replyTo: replyTo ?? '' }, 'text', text, replyTo)
  }

  sendGif(t: Target, gif: Record<string, unknown>) {
    return this.sendNew(t, { type: 'gif', ...gif }, 'gif', JSON.stringify(gif), null)
  }

  sendInvite(t: Target, invite: Record<string, unknown>) {
    const inv = { ...invite, expiresAt: Date.now() + INVITE_TTL_MS }
    return this.sendNew(t, { type: 'invite', ...inv }, 'invite', JSON.stringify(inv), null)
  }

  async sendFile(t: Target, file: File): Promise<MessageRow> {
    if (file.size > 25 * 1024 * 1024) throw new SendError('Files can be up to 25 MB.')
    const sealed = Kryo.sealFile(new Uint8Array(await file.arrayBuffer()))
    const key = sealed.slice(0, 32)
    const sha = sealed.slice(32, 64)
    const ct = sealed.slice(64)
    const ticket = await this.ask('attachmentTicket', { size: ct.length })
    const up = await fetch(`${GATEWAY_HTTP}/v1/attachments/${ticket.id}?t=${ticket.uploadToken}&s=${ct.length}`, { method: 'PUT', body: ct })
    if (!up.ok) throw new SendError(`The upload was refused (${up.status}).`)
    const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('')
    const name = file.name.replace(/[\u0000-\u001f/\\]/g, '').slice(0, 200) || 'file'
    const meta = { id: ticket.id, token: ticket.downloadToken, key: hex(key), sha256: hex(sha), name, mime: file.type || 'application/octet-stream', size: file.size, width: 0, height: 0 }
    return this.sendNew(t, { type: 'file', ...meta }, 'file', JSON.stringify(meta), null)
  }

  async fetchFile(msgId: string): Promise<{ name: string; mime: string; bytes: Uint8Array }> {
    const row = await this.store.message(msgId)
    if (!row || row.kind !== 'file' || row.deleted) throw new Error('That message has no file.')
    const f = JSON.parse(row.body) as { id: string; token: string; key: string; sha256: string; name: string; mime: string }
    const res = await fetch(`${GATEWAY_HTTP}/v1/attachments/${f.id}?t=${f.token}`)
    if (res.status === 404) throw new Error('This file is no longer available (files are kept for 30 days).')
    if (!res.ok) throw new Error(`The download was refused (${res.status}).`)
    try {
      return { name: f.name, mime: f.mime, bytes: Kryo.openFile(f.key, f.sha256, new Uint8Array(await res.arrayBuffer())) }
    } catch {
      throw new Error('The file did not check out; it may have been changed.')
    }
  }

  private async control(t: Target, body: object) {
    const c = this.content(t, body)
    Kryo.validate(JSON.stringify(c), this.meSupporter)
    const { users, group } = await this.recipients(t)
    await this.deliver(users, c, false, group)
  }

  private async ownRecent(msgId: string): Promise<MessageRow> {
    const r = await this.store.message(msgId)
    if (!r || !r.outgoing || r.deleted) throw new SendError('That message cannot be changed.')
    if (Date.now() - r.sentAt > EDIT_WINDOW_MS) throw new SendError('Messages can be changed for 24 hours.')
    return r
  }

  async edit(msgId: string, text: string): Promise<MessageRow> {
    const r = await this.ownRecent(msgId)
    if (r.kind !== 'text') throw new SendError('Only text can be edited.')
    const t = targetOf(r.conversationId, this.userId)
    if (!t) throw new SendError('Not a conversation you are in.')
    await this.control(t, { type: 'edit', target: msgId, text })
    const u = (await this.store.update(msgId, (x) => ({ ...x, body: text, editedAt: Date.now() })))!
    this.emit('chat-updated', this.shown(u))
    return this.shown(u)
  }

  async remove(msgId: string) {
    const r = await this.ownRecent(msgId)
    const t = targetOf(r.conversationId, this.userId)
    if (!t) throw new SendError('Not a conversation you are in.')
    await this.control(t, { type: 'delete', target: msgId })
    const u = await this.store.update(msgId, (x) => ({ ...x, deleted: true, body: '' }))
    if (u) this.emit('chat-updated', this.shown(u))
  }

  async react(msgId: string, emoji: string, remove: boolean) {
    const r = await this.store.message(msgId)
    const t = r && targetOf(r.conversationId, this.userId)
    if (!t) throw new SendError('No such message.')
    await this.control(t, { type: 'reaction', target: msgId, emoji, remove })
    const u = await this.applyReaction(msgId, this.userId, emoji, remove)
    if (u) this.emit('chat-updated', this.shown(u))
  }

  private applyReaction(msgId: string, user: string, emoji: string, remove: boolean) {
    return this.store.update(msgId, (x) => {
      const list = x.reactions.map((r) => ({ ...r, userIds: r.userIds.filter((u) => u !== user || r.emoji !== emoji) }))
      if (!remove) {
        const hit = list.find((r) => r.emoji === emoji)
        if (hit) hit.userIds.push(user)
        else list.push({ emoji, userIds: [user] })
      }
      return { ...x, reactions: list.filter((r) => r.userIds.length > 0) }
    })
  }

  async typing(t: Target, active: boolean) {
    if (!(await this.settings()).typing || ('dm' in t && this.awaiting(t.dm))) return
    const { users, group } = await this.recipients(t)
    await this.deliver(users.filter((u) => u !== this.userId), this.content(t, { type: 'typing', active }), true, group)
  }

  async markRead(t: Target) {
    const conv = convOf(t, this.userId)
    const ids = await this.store.markRead(conv)
    if (ids.length === 0 || !this.online) return
    const body = { type: 'receipt', receiptKind: 'read', msgIds: ids }
    if ('group' in t) {
      await this.deliver([this.userId], this.content(t, body), false, t.group).catch(() => {})
      return
    }
    if (!(await this.settings()).readReceipts || this.awaiting(t.dm)) return
    await this.deliver([t.dm, this.userId], this.content(t, body), false, null).catch(() => {})
  }

  async sendCall(peer: string, callId: string, callKind: string, payload: string) {
    if (this.awaiting(peer)) throw new SendError('Accept their message request first.')
    const c = this.content({ dm: peer }, { type: 'call', callId, callKind, payload })
    Kryo.validate(JSON.stringify(c), this.meSupporter)
    // Picking up or turning it down here stops it ringing on this account's other devices.
    const mine = callKind === 'answer' || callKind === 'decline'
    await this.deliver(mine ? [peer, this.userId] : [peer], c, true, null)
  }

  /** The record of a finished call, in the conversation for both people (the caller sends it). */
  async callLog(peer: string, callId: string, answered: boolean, seconds: number) {
    const secs = answered ? Math.min(24 * 3600, Math.max(0, Math.round(seconds))) : 0
    const body = { type: 'callLog', callId, answered, seconds: secs }
    return this.sendNew({ dm: peer }, body, 'call', JSON.stringify({ callId, answered, seconds: secs, caller: this.userId }), null)
  }

  /** "This device is in use": to this account's other devices online now, never stored. */
  async active() {
    await this.deliver([this.userId], this.content({ dm: this.userId }, { type: 'active' }), true, null)
  }

  private async resendPending() {
    for (const row of await this.store.all()) {
      if (!row.outgoing || row.status !== 'sending') continue
      const t = targetOf(row.conversationId, this.userId)
      if (!t) continue
      let body: Record<string, unknown>
      if (row.kind === 'text') body = { type: 'text', text: row.body, replyTo: row.replyTo ?? '' }
      else if (row.kind === 'call') {
        const { callId, answered, seconds } = JSON.parse(row.body) as { callId: string; answered: boolean; seconds: number }
        body = { type: 'callLog', callId, answered, seconds }
      } else body = { type: row.kind, ...(JSON.parse(row.body) as object) }
      try {
        const { users, group } = await this.recipients(t)
        await this.deliver(users, this.content(t, body, row.msgId, row.sentAt), false, group)
        const u = await this.store.advance(row.msgId, 'sent')
        if (u) this.emit('chat-updated', this.shown(u))
      } catch {
        if (!this.online) return
        const u = await this.store.advance(row.msgId, 'failed')
        if (u) this.emit('chat-updated', this.shown(u))
      }
    }
  }

  // ---- receiving --------------------------------------------------------------

  private async handleDelivery(d: Frame) {
    const envelope = b64ToBytes(d.envelope)
    let r = JSON.parse(this.kryo.decrypt(envelope)) as { ok?: { sender: { userId: string; deviceId: string }; content: any }; error?: string; userId?: string }
    if (r.error === 'unknownSender' && r.userId) {
      try {
        await this.devicesOf(r.userId, true)
        r = JSON.parse(this.kryo.decrypt(envelope))
      } catch {
        r = { error: 'unknownSender' }
      }
    }
    if (r.ok) await this.apply(r.ok.sender.userId, r.ok.sender.deviceId, r.ok.content).catch(() => {})
    // Stored first, then the state saved, then acknowledged.
    await this.persist()
    if (d.seq && d.seq !== '0' && this.ws?.readyState === WebSocket.OPEN) this.ws.send(this.kryo.request(0, 'ack', JSON.stringify({ upToSeq: d.seq })))
  }

  private async apply(sender: string, senderDevice: string, c: any) {
    const me = this.userId
    const t = targetOf(c.conversationId, me)
    if (!t) return
    if ('dm' in t && sender !== me && t.dm !== sender) return
    if ('group' in t && !(await this.isMember(t.group, sender))) return
    const fromMe = sender === me
    const b = c.body
    const sameConv = async (id: string) => (await this.store.message(id))?.conversationId === c.conversationId
    switch (b.type) {
      case 'text':
      case 'gif':
      case 'invite':
      case 'file':
      case 'callLog': {
        if (b.type === 'invite' && !/^[a-z0-9-]{1,120}$/.test(b.slug ?? '')) return
        if (b.type === 'callLog' && !('dm' in t)) return
        const { type, ...rest } = b
        void type
        // A call record keeps who made the call.
        if (b.type === 'callLog') rest.caller = sender
        const row: MessageRow = {
          msgId: c.msgId,
          conversationId: c.conversationId,
          peerUserId: 'dm' in t ? t.dm : null,
          senderUser: sender,
          senderDevice,
          outgoing: fromMe,
          sentAt: c.sentAt,
          receivedAt: Date.now(),
          kind: b.type === 'callLog' ? 'call' : b.type,
          body: b.type === 'text' ? b.text : JSON.stringify(rest),
          replyTo: b.type === 'text' && b.replyTo && b.replyTo.length === 32 ? b.replyTo : null,
          editedAt: null,
          deleted: false,
          status: fromMe ? 'sent' : 'received',
          reactions: [],
        }
        if (!(await this.store.insert(row))) return
        const shown = this.shown(row)
        this.emit('chat-message', shown)
        if (fromMe) return
        this.emit('chat-unread', { conversationId: c.conversationId })
        // A call record follows the call, which already rang.
        if (!this.people.get(sender)?.muted && b.type !== 'callLog') void this.notify(sender, shown, 'group' in t ? t.group : null)
        if (this.awaiting(sender) || 'group' in t) return
        await this.deliver([sender, me], this.content(t, { type: 'receipt', receiptKind: 'delivered', msgIds: [c.msgId] }), false, null).catch(() => {})
        return
      }
      case 'receipt': {
        if (fromMe) {
          if (b.receiptKind === 'read') {
            await this.store.markRead(c.conversationId)
            this.emit('chat-read', { conversationId: c.conversationId })
          }
          return
        }
        const status = b.receiptKind === 'read' ? ((await this.settings()).readReceipts ? 'read' : null) : 'delivered'
        if (!status) return
        for (const id of (b.msgIds as string[]).slice(0, 500)) {
          const m = await this.store.message(id)
          if (!m || !m.outgoing || m.conversationId !== c.conversationId) continue
          const u = await this.store.advance(id, status)
          if (u) this.emit('chat-updated', this.shown(u))
        }
        return
      }
      case 'typing':
        if (!fromMe && (await this.settings()).typing) this.emit('chat-typing', { conversationId: c.conversationId, userId: sender, active: b.active })
        return
      case 'edit': {
        if (!(await sameConv(b.target))) return
        const u = await this.store.update(b.target, (x) => (x.senderUser === sender && !x.deleted ? { ...x, body: b.text, editedAt: c.sentAt } : null))
        if (u) this.emit('chat-updated', this.shown(u))
        return
      }
      case 'delete': {
        if (!(await sameConv(b.target))) return
        const u = await this.store.update(b.target, (x) => (x.senderUser === sender ? { ...x, deleted: true, body: '' } : null))
        if (u) this.emit('chat-updated', this.shown(u))
        return
      }
      case 'reaction': {
        if (!(await sameConv(b.target))) return
        const u = await this.applyReaction(b.target, sender, b.emoji, b.remove)
        if (u) this.emit('chat-updated', this.shown(u))
        return
      }
      case 'groupMeta':
        if ('group' in t && typeof b.name === 'string' && b.name.trim()) {
          await this.store.set(`group-name-${t.group}`, b.name.trim().slice(0, 64))
          this.emit('chat-group-changed', { groupId: t.group })
        }
        return
      case 'active':
        if (fromMe) this.activeElsewhereUntil = Date.now() + 75_000
        return
      case 'call':
        // Picked up or turned down on another of this account's devices.
        if (fromMe && 'dm' in t && (b.callKind === 'answer' || b.callKind === 'decline')) {
          this.emit('chat-call', { userId: t.dm, callId: b.callId, kind: 'elsewhere', payload: '' })
          return
        }
        if (!fromMe && 'dm' in t && !this.awaiting(sender)) {
          this.emit('chat-call', { userId: sender, callId: b.callId, kind: b.callKind, payload: b.payload })
          // One notice per call, however many offers it takes.
          if (b.callKind === 'offer' && !this.rung.has(b.callId)) {
            this.rung.add(b.callId)
            if (document.hidden) void this.notifyText(`${this.people.get(sender)?.name ?? 'Someone'} is calling`, null)
          }
        }
        return
    }
  }

  private async notify(sender: string, row: MessageRow, group: string | null) {
    if (!document.hidden || Date.now() < this.activeElsewhereUntil) return
    const mode = (await this.settings()).notificationContent
    const who = this.people.get(sender)?.name ?? 'New message'
    const title = mode === 'none' ? 'New message on Kryoto' : group ? `${who} in ${await this.groupName(group)}` : who
    const body = mode !== 'full' ? null : row.kind === 'text' ? row.body.slice(0, 140) : row.kind === 'gif' ? 'Sent a GIF' : row.kind === 'file' ? 'Sent a file' : 'Sent a game invite'
    await this.notifyText(title, body)
  }

  private async notifyText(title: string, body: string | null) {
    // Inside kryo.to the site shows it: this frame may never notify itself.
    if (EMBEDDED) {
      toSite({ type: 'notify', title, body })
      return
    }
    if (typeof Notification === 'undefined' || Notification.permission !== 'granted') return
    try {
      new Notification(title, body ? { body } : undefined)
    } catch {
      // some browsers only allow notifications from a service worker
    }
  }

  // ---- reading ---------------------------------------------------------------

  async conversations() {
    const byConv = new Map<string, MessageRow[]>()
    for (const r of await this.store.all()) {
      const l = byConv.get(r.conversationId) ?? []
      l.push(r)
      byConv.set(r.conversationId, l)
    }
    return [...byConv.entries()]
      .map(([id, rows]) => {
        rows.sort((a, b) => a.sentAt - b.sentAt)
        const last = rows[rows.length - 1]!
        return {
          id,
          peerUserId: targetOf(id, this.userId) && 'dm' in targetOf(id, this.userId)! ? (targetOf(id, this.userId) as { dm: string }).dm : null,
          lastAt: last.sentAt,
          unread: rows.filter((r) => !r.outgoing && r.status === 'received' && !r.deleted).length,
          last: this.shown(last),
        }
      })
      .sort((a, b) => b.lastAt - a.lastAt)
  }

  async messages(t: Target, before: number | null) {
    return (await this.store.messages(convOf(t, this.userId), before)).map((m) => this.shown(m))
  }

  async search(q: string) {
    const n = q.trim().toLowerCase()
    if (n.length < 2) return []
    return (await this.store.all())
      .filter((r) => r.kind === 'text' && !r.deleted && r.body.toLowerCase().includes(n))
      .sort((a, b) => b.sentAt - a.sentAt)
      .slice(0, 50)
      .map((m) => this.shown(m))
  }

  // ---- groups ------------------------------------------------------------------

  private remember(g: Frame) {
    this.groups.set(g.groupId, g.members)
    return g.members as { userId: string; role: string }[]
  }

  private async groupMembers(id: string, refresh: boolean) {
    if (!refresh && this.groups.has(id)) return this.groups.get(id)!
    try {
      return this.remember(await this.ask('groupGet', { groupId: id }))
    } catch (e) {
      if (String(e).includes('No such group')) this.groups.delete(id)
      return this.groups.get(id) ?? null
    }
  }

  private async isMember(group: string, user: string) {
    if (this.groups.get(group)?.some((m) => m.userId === user)) return true
    return (await this.groupMembers(group, true))?.some((m) => m.userId === user) ?? false
  }

  async groupName(id: string) {
    return (await this.store.get<string>(`group-name-${id}`)) || 'Group chat'
  }

  private async view(id: string, members: { userId: string; role: string }[]) {
    return { id, name: await this.groupName(id), members, myRole: members.find((m) => m.userId === this.userId)?.role ?? '' }
  }

  async groupList() {
    const f = await this.ask('groupList')
    const out = []
    this.groups.clear()
    for (const g of f.groups as Frame[]) out.push(await this.view(g.groupId, this.remember(g)))
    return out
  }

  private async sendGroupName(id: string, name: string) {
    const n = name.trim().slice(0, 64)
    await this.store.set(`group-name-${id}`, n)
    await this.control({ group: id }, { type: 'groupMeta', name: n })
  }

  async groupCreate(name: string, members: string[]) {
    if (!name.trim()) throw new SendError('Give the group a name.')
    const g = await this.ask('groupCreate', { memberIds: members })
    const m = this.remember(g)
    await this.sendGroupName(g.groupId, name)
    return this.view(g.groupId, m)
  }

  async groupRename(id: string, name: string) {
    if (!name.trim()) throw new SendError('Give the group a name.')
    await this.sendGroupName(id, name)
    this.emit('chat-group-changed', { groupId: id })
  }

  async groupAdd(id: string, users: string[]) {
    const m = this.remember(await this.ask('groupAdd', { groupId: id, userIds: users }))
    await this.sendGroupName(id, await this.groupName(id)).catch(() => {})
    return this.view(id, m)
  }

  async groupRemove(id: string, user: string) {
    const f = await this.ask('groupRemove', { groupId: id, userId: user })
    if (f.type === 'group') this.remember(f)
    else this.groups.delete(id)
    this.emit('chat-group-changed', { groupId: id })
  }

  private async onGroupChanged(id: string) {
    await this.groupMembers(id, true)
    this.emit('chat-group-changed', { groupId: id })
  }

  // ---- verification, backup, devices, identity ------------------------------

  async verifyInfo(peer: string) {
    if (this.online) await this.devicesOf(peer, true).catch(() => {})
    return JSON.parse(this.kryo.verifyInfo(peer)) as unknown
  }

  async verifyMark(peer: string, verified: boolean) {
    this.kryo.markVerified(peer, verified)
    await this.persist()
    await this.backupRefresh()
  }

  async identityAck(peer: string) {
    this.kryo.acknowledgeChange(peer)
    await this.persist()
  }

  // ---- web push ------------------------------------------------------------
  //
  // With the tab closed, the gateway sends this browser an empty push when a
  // message arrives (kryoto-gateway src/push.rs) and public/chat-sw.js shows
  // "New message". Nothing about the message passes through the push service.

  private async pushRegistration(): Promise<ServiceWorkerRegistration | null> {
    if (!('serviceWorker' in navigator) || !('PushManager' in window)) return null
    return navigator.serviceWorker.register('/chat-sw.js', { scope: '/' })
  }

  async pushState(): Promise<{ supported: boolean; permission: NotificationPermission | 'unsupported'; on: boolean }> {
    const supported = !!this.pushKey && 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window
    if (!supported) return { supported: false, permission: 'unsupported', on: false }
    const reg = await navigator.serviceWorker.getRegistration('/')
    const sub = reg ? await reg.pushManager.getSubscription() : null
    return { supported, permission: Notification.permission, on: !!sub && Notification.permission === 'granted' }
  }

  /** Ask for permission (inside the click), subscribe, and hand the endpoint to the gateway. */
  async pushEnable(): Promise<void> {
    if (!this.pushKey) throw new Error('Notifications are not available on this chat server.')
    const permission = await Notification.requestPermission()
    if (permission !== 'granted') throw new Error('Notifications are blocked for chat.kryo.to in this browser.')
    const reg = await this.pushRegistration()
    if (!reg) throw new Error('This browser cannot receive notifications.')
    await navigator.serviceWorker.ready
    const key = hexToBytes(this.pushKey)
    let sub = await reg.pushManager.getSubscription()
    // A subscription made with another server key cannot be used: start over.
    if (sub && !sameKey(sub.options.applicationServerKey, key)) {
      await sub.unsubscribe()
      sub = null
    }
    sub ??= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key as BufferSource })
    await this.ask('pushSubscribe', { endpoint: sub.endpoint })
    await this.store.set('push-endpoint', sub.endpoint)
  }

  async pushDisable(): Promise<void> {
    const reg = 'serviceWorker' in navigator ? await navigator.serviceWorker.getRegistration('/') : undefined
    const sub = reg ? await reg.pushManager.getSubscription() : null
    await sub?.unsubscribe()
    await this.store.set('push-endpoint', '')
    await this.ask('pushUnsubscribe').catch(() => {})
  }

  /** After each connect: a browser may have rotated its endpoint since we last told the gateway. */
  private async pushResync() {
    try {
      if (!this.pushKey || !('serviceWorker' in navigator) || Notification.permission !== 'granted') return
      const reg = await navigator.serviceWorker.getRegistration('/')
      const sub = reg ? await reg.pushManager.getSubscription() : null
      if (!sub) return
      if ((await this.store.get<string>('push-endpoint')) === sub.endpoint) return
      await this.ask('pushSubscribe', { endpoint: sub.endpoint })
      await this.store.set('push-endpoint', sub.endpoint)
    } catch {
      // Next connect tries again.
    }
  }

  async backupStatus() {
    const f = await this.ask('backupGet')
    const code = await this.store.get<string>('backup-code')
    return { exists: !!f.blob, updatedAtMs: f.updatedAt ?? 0, keptHere: !!code }
  }

  async backupCreate(): Promise<string> {
    const b = JSON.parse(this.kryo.backupCreate()) as { code: string; blob: string }
    await this.ask('backupPut', { blob: b.blob })
    await this.store.set('backup-code', b.code)
    return b.code
  }

  private async backupRefresh() {
    const code = await this.store.get<string>('backup-code')
    if (!code || !this.online) return
    await this.ask('backupPut', { blob: this.kryo.backupReseal(code) }).catch(() => {})
  }

  async backupDelete() {
    await this.ask('backupDelete')
    await this.store.del('backup-code')
  }

  async myDevices() {
    const f = await this.ask('listMyDevices')
    return (f.devices as Frame[]).map((d) => ({ ...d, current: d.deviceId === this.kryo.deviceId() }))
  }

  async revokeDevice(id: string) {
    if (id === this.kryo.deviceId()) throw new SendError('Use "Remove from this browser" for this one.')
    await this.ask('revokeDevice', { deviceId: id })
    this.kryo.forgetDevices(this.userId)
  }

  /** Short connection-less steps happen on the live socket here: restore and reset run while needsLink, when the socket is closed; reconnect to do them. */
  private async oneOff<T>(work: () => Promise<T>): Promise<T> {
    // Reconnect in a mode that stops right after "ready" (settle is skipped
    // because the identity is about to change), do the work, then go live.
    this.stop()
    await new Promise((r) => setTimeout(r, 50))
    const ws = new WebSocket(GATEWAY_WS)
    ws.binaryType = 'arraybuffer'
    this.ws = ws
    await new Promise<void>((resolve, reject) => {
      ws.onerror = () => reject(new Error('Could not reach the chat server.'))
      ws.onopen = () => ws.send(this.kryo.hello(this.token))
      ws.onmessage = (ev) => {
        const f = JSON.parse(Kryo.decode(new Uint8Array(ev.data as ArrayBuffer))) as Frame
        if (f.type === 'challenge') ws.send(this.kryo.proof(b64ToBytes(f.nonce)))
        else if (f.type === 'ready') {
          ws.onmessage = (e2) => {
            const g = JSON.parse(Kryo.decode(new Uint8Array(e2.data as ArrayBuffer))) as Frame
            if (g.requestId) {
              this.pending.get(g.requestId)?.(g)
              this.pending.delete(g.requestId)
            }
          }
          resolve()
        } else if (f.type === 'error') reject(new Error(f.message))
      }
    })
    this.status = { state: 'online', userId: this.userId, deviceId: this.kryo.deviceId() ?? '' }
    try {
      return await work()
    } finally {
      ws.close()
      await this.persist()
      this.run()
    }
  }

  restore(code: string) {
    return this.oneOff(async () => {
      const f = await this.ask('backupGet')
      if (!f.blob) throw new SendError('Your account has no key backup. Start a new chat identity instead, or turn on a backup on your other device first.')
      this.kryo.restoreBackup(code, f.blob)
      await this.store.set('backup-code', code)
    })
  }

  resetIdentity() {
    return this.oneOff(async () => {
      this.kryo.resetIdentity()
      await this.persist()
      await this.ask('publishMasterKey')
      await this.ask('certify')
      await this.ask('backupDelete').catch(() => {})
      await this.store.del('backup-code')
    })
  }

  /** Take this browser out of chat: revoke the device, forget everything. */
  async removeDevice() {
    const id = this.kryo.deviceId()
    if (id && this.online) await this.ask('revokeDevice', { deviceId: id }).catch(() => {})
    this.stop()
    await api(this.token, 'POST', '/api/auth/logout').catch(() => {})
    await Store.wipe()
  }

  async exportHistory() {
    const rows = (await this.store.all()).filter((r) => !r.deleted)
    const convs = new Map<string, MessageRow[]>()
    for (const r of rows) convs.set(r.conversationId, [...(convs.get(r.conversationId) ?? []), r])
    return {
      account: this.userId,
      exportedAt: new Date().toISOString(),
      conversations: await Promise.all(
        [...convs.entries()].map(async ([id, list]) => {
          const t = targetOf(id, this.userId)
          return {
            with: t && 'dm' in t ? (this.people.get(t.dm)?.name ?? t.dm) : t ? await this.groupName(t.group) : id,
            messages: list
              .sort((a, b) => a.sentAt - b.sentAt)
              .map((m) => ({ sentAt: m.sentAt, from: m.outgoing ? 'me' : (this.people.get(m.senderUser)?.name ?? m.senderUser), kind: m.kind, text: this.shown(m).body })),
          }
        }),
      ),
    }
  }

  async reportMessages(peer: string, ids: string[], conversation: string | null) {
    const t = conversation ? parseTarget(conversation) : { dm: peer }
    const wanted = new Set(ids)
    return (await this.store.messages(convOf(t, this.userId), null, 500))
      .filter((m) => wanted.has(m.msgId) && !m.deleted && (m.outgoing || m.senderUser === peer))
      .map((m) => ({ sentAt: m.sentAt, mine: m.outgoing, kind: m.kind, text: this.shown(m).body }))
  }
}

function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2)
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  return out
}

function sameKey(a: ArrayBuffer | null | undefined, b: Uint8Array): boolean {
  if (!a) return false
  const x = new Uint8Array(a)
  return x.length === b.length && x.every((v, i) => v === b[i])
}
