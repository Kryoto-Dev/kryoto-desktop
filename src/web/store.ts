/**
 * The web chat's storage: IndexedDB, with every secret and every message
 * encrypted (AES-GCM) under a key WebCrypto created as non-extractable. The
 * page can use that key but never read it, so a copy of the browser profile
 * is not enough to read chats. (A compromised page could still use it while
 * open - that is the web's residual risk, stated when chat is turned on.)
 *
 * Stores:
 * - `meta`: the CryptoKey itself (structured clone keeps it non-extractable).
 * - `kv`: encrypted values (device state, session token, settings).
 * - `messages`: one row per message: `msgId`, `conv` and `sentAt` in the
 *   clear for indexing, everything else encrypted.
 */

export type Reaction = { emoji: string; userIds: string[] }

export type MessageRow = {
  msgId: string
  conversationId: string
  peerUserId: string | null
  senderUser: string
  senderDevice: string
  outgoing: boolean
  sentAt: number
  receivedAt: number
  kind: 'text' | 'gif' | 'invite' | 'file' | 'call'
  body: string
  replyTo: string | null
  editedAt: number | null
  deleted: boolean
  status: 'sending' | 'sent' | 'delivered' | 'read' | 'failed' | 'received'
  reactions: Reaction[]
}

const DB = 'kryoto-chat'
const STATUS_RANK: Record<MessageRow['status'], number> = { sending: 0, failed: 0, sent: 1, delivered: 2, read: 3, received: 0 }

function req<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((res, rej) => {
    r.onsuccess = () => res(r.result)
    r.onerror = () => rej(r.error)
  })
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((res, rej) => {
    const r = indexedDB.open(DB, 1)
    r.onupgradeneeded = () => {
      const db = r.result
      db.createObjectStore('meta')
      db.createObjectStore('kv')
      const m = db.createObjectStore('messages', { keyPath: 'msgId' })
      m.createIndex('conv', ['conv', 'sentAt'])
    }
    r.onsuccess = () => res(r.result)
    r.onerror = () => rej(r.error)
  })
}

const enc = new TextEncoder()
const dec = new TextDecoder()

export class Store {
  private constructor(
    private db: IDBDatabase,
    private key: CryptoKey,
  ) {}

  static async open(): Promise<Store> {
    const db = await openDb()
    const tx = db.transaction('meta', 'readonly')
    let key = (await req(tx.objectStore('meta').get('key'))) as CryptoKey | undefined
    if (!key) {
      key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt'])
      await req(db.transaction('meta', 'readwrite').objectStore('meta').put(key, 'key'))
    }
    return new Store(db, key)
  }

  private async seal(value: unknown): Promise<ArrayBuffer> {
    const iv = crypto.getRandomValues(new Uint8Array(12))
    const ct = await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, this.key, enc.encode(JSON.stringify(value)))
    const out = new Uint8Array(12 + ct.byteLength)
    out.set(iv)
    out.set(new Uint8Array(ct), 12)
    return out.buffer
  }

  private async open<T>(data: ArrayBuffer): Promise<T> {
    const b = new Uint8Array(data)
    const pt = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: b.slice(0, 12) }, this.key, b.slice(12))
    return JSON.parse(dec.decode(pt)) as T
  }

  async get<T>(name: string): Promise<T | null> {
    const v = (await req(this.db.transaction('kv', 'readonly').objectStore('kv').get(name))) as ArrayBuffer | undefined
    return v ? this.open<T>(v) : null
  }

  async set(name: string, value: unknown): Promise<void> {
    const sealed = await this.seal(value)
    await req(this.db.transaction('kv', 'readwrite').objectStore('kv').put(sealed, name))
  }

  async del(name: string): Promise<void> {
    await req(this.db.transaction('kv', 'readwrite').objectStore('kv').delete(name))
  }

  // ---- messages ----

  private async putRow(row: MessageRow): Promise<void> {
    const data = await this.seal(row)
    await req(this.db.transaction('messages', 'readwrite').objectStore('messages').put({ msgId: row.msgId, conv: row.conversationId, sentAt: row.sentAt, data }))
  }

  async message(msgId: string): Promise<MessageRow | null> {
    const r = (await req(this.db.transaction('messages', 'readonly').objectStore('messages').get(msgId))) as { data: ArrayBuffer } | undefined
    return r ? this.open<MessageRow>(r.data) : null
  }

  /** Insert a new message; false if that id is already stored. */
  async insert(row: MessageRow): Promise<boolean> {
    if (await this.message(row.msgId)) return false
    await this.putRow(row)
    return true
  }

  async update(msgId: string, change: (r: MessageRow) => MessageRow | null): Promise<MessageRow | null> {
    const row = await this.message(msgId)
    if (!row) return null
    const next = change(row)
    if (!next) return null
    await this.putRow(next)
    return next
  }

  /** Move a status forward only (sent -> delivered -> read). */
  advance(msgId: string, status: MessageRow['status']): Promise<MessageRow | null> {
    return this.update(msgId, (r) => {
      if (status === 'failed' || status === 'sent') {
        return r.status === 'sending' || r.status === 'failed' ? { ...r, status } : null
      }
      return STATUS_RANK[status] > STATUS_RANK[r.status] ? { ...r, status } : null
    })
  }

  private async rows(conv: string): Promise<MessageRow[]> {
    const idx = this.db.transaction('messages', 'readonly').objectStore('messages').index('conv')
    const raw = (await req(idx.getAll(IDBKeyRange.bound([conv, 0], [conv, Number.MAX_SAFE_INTEGER])))) as { data: ArrayBuffer }[]
    return Promise.all(raw.map((r) => this.open<MessageRow>(r.data)))
  }

  /** Newest `limit` messages of a conversation (before `before`), oldest first. */
  async messages(conv: string, before: number | null, limit = 60): Promise<MessageRow[]> {
    const all = (await this.rows(conv)).filter((r) => before == null || r.sentAt < before)
    all.sort((a, b) => a.sentAt - b.sentAt || a.msgId.localeCompare(b.msgId))
    return all.slice(-limit)
  }

  async all(): Promise<MessageRow[]> {
    const raw = (await req(this.db.transaction('messages', 'readonly').objectStore('messages').getAll())) as { data: ArrayBuffer }[]
    return Promise.all(raw.map((r) => this.open<MessageRow>(r.data)))
  }

  /** Mark incoming messages of a conversation read; returns their ids. */
  async markRead(conv: string): Promise<string[]> {
    const ids: string[] = []
    for (const r of await this.rows(conv)) {
      if (!r.outgoing && r.status === 'received' && !r.deleted) {
        ids.push(r.msgId)
        await this.putRow({ ...r, status: 'read' })
      }
    }
    return ids
  }

  /** Forget everything on this browser (chat removed). */
  static async wipe(): Promise<void> {
    await new Promise<void>((res) => {
      const r = indexedDB.deleteDatabase(DB)
      r.onsuccess = r.onerror = r.onblocked = () => res()
    })
  }
}
