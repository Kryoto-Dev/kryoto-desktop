import { call, isTauri, isWeb, on } from '@/lib/bridge'
import { syncEmojiUsage } from '@/rich/emoji-usage'

/**
 * Managing friends from the friends list itself: requests, answers, removing,
 * blocking, pins and mutes. kryo.to decides everything; these only ask.
 *
 * In the app the call runs in the Store's kryo.to page with the session the
 * player signed in with there (the native side never holds that cookie), and
 * the answer comes back as `store-friend-done`. The web chat sends the same
 * request with its own token, and the browser preview fakes it.
 */
export type FriendAction =
  | { kind: 'request'; username: string }
  | { kind: 'respond'; requestId: string; answer: 'accept' | 'decline' | 'cancel' }
  | { kind: 'remove'; userId: string }
  | { kind: 'block'; userId: string }
  | { kind: 'unblock'; userId: string }
  | { kind: 'favourite'; userId: string; on: boolean }
  | { kind: 'mute'; userId: string; on: boolean }
  | { kind: 'invite' }

export type FriendFetch = { method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'; path: string; body?: Record<string, unknown> }

const id = (v: string) => {
  if (!/^\d{1,20}$/.test(v)) throw new Error('Not an account.')
  return v
}

/** The kryo.to route behind each action (also what the native side allows). */
export function friendFetch(a: FriendAction): FriendFetch {
  switch (a.kind) {
    case 'request':
      return { method: 'POST', path: '/api/friends/requests', body: { username: a.username.trim().replace(/^@/, '') } }
    case 'respond':
      return { method: 'POST', path: `/api/friends/requests/${id(a.requestId)}`, body: { action: a.answer } }
    case 'remove':
      return { method: 'DELETE', path: `/api/friends/${id(a.userId)}` }
    case 'block':
      return { method: 'POST', path: `/api/blocks/${id(a.userId)}` }
    case 'unblock':
      return { method: 'DELETE', path: `/api/blocks/${id(a.userId)}` }
    case 'favourite':
      return { method: 'PATCH', path: `/api/friends/${id(a.userId)}`, body: { favourite: a.on } }
    case 'mute':
      return a.on ? { method: 'POST', path: `/api/mutes/${id(a.userId)}`, body: {} } : { method: 'DELETE', path: `/api/mutes/${id(a.userId)}` }
    case 'invite':
      return { method: 'POST', path: '/api/friends/invites' }
  }
}

type Done = { id: string; ok: boolean; status: number; data?: Record<string, unknown> | null }

/**
 * One request through the Store's kryo.to page, with its session, answered
 * by `store-friend-done`. The native side allows only the friends list's
 * routes, the account's play-time read and whether it wrote a game up (lib.rs `friend_route_allowed`).
 */
export function viaStore(f: FriendFetch): Promise<Record<string, unknown>> {
  const ticket = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
  return new Promise((resolve, reject) => {
    let stop: (() => void) | undefined
    const timer = window.setTimeout(() => {
      stop?.()
      reject(new Error('kryo.to did not answer. Check that you are signed in in the Store, then try again.'))
    }, 20_000)
    void on<Done>('store-friend-done', (r) => {
      if (r.id !== ticket) return
      window.clearTimeout(timer)
      stop?.()
      const data = r.data ?? {}
      if (r.ok) resolve(data)
      else reject(new Error(typeof data.error === 'string' ? data.error : r.status === 0 ? 'Could not reach kryo.to.' : `kryo.to answered ${r.status}.`))
    })
      .then((fn) => {
        stop = fn
        return call('store_friend_fetch', { id: ticket, method: f.method, path: f.path, body: f.body ?? null })
      })
      .catch((e: unknown) => {
        window.clearTimeout(timer)
        stop?.()
        reject(e instanceof Error ? e : new Error(String(e)))
      })
  })
}

/** Run one action; resolves kryo.to's answer, rejects with its error in words. */
export async function friendAction(a: FriendAction): Promise<Record<string, unknown>> {
  const f = friendFetch(a)
  const data = isTauri() && !isWeb() ? await viaStore(f) : await call<Record<string, unknown>>('friends_fetch', f)
  // The lists change with every action: ask for them again (the Store page
  // also does this itself once its request settles).
  void call('store_refresh_account').catch(() => {})
  return data ?? {}
}

let usageFor: string | null = null

/**
 * The emoji picker's "Frequently used" row follows the signed-in account
 * (rich/emoji-usage.ts, kryo.to `/api/account/emoji-usage`), through the same
 * door as the friends list: the Store's page in the app, the token in the web
 * chat. Once per account; signing out stops saving.
 */
export function followEmojiUsage(username: string | null): void {
  if (username === usageFor) return
  usageFor = username
  if (!username) {
    syncEmojiUsage(null)
    return
  }
  const go = (f: FriendFetch) => (isTauri() && !isWeb() ? viaStore(f) : call<Record<string, unknown>>('friends_fetch', f))
  syncEmojiUsage({
    load: () =>
      go({ method: 'GET', path: '/api/account/emoji-usage' }).then((d) =>
        d && typeof d.tally === 'object' && d.tally ? (d.tally as Record<string, number>) : null,
      ),
    save: (tally) => go({ method: 'PUT', path: '/api/account/emoji-usage', body: { tally } }),
  })
}
