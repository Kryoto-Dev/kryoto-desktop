import { API_BASE } from './api'

/**
 * The web chat inside kryo.to (its Friends & chat panel), as a frame.
 *
 * The chat keeps its own origin on purpose (kryo.to runs ad scripts, and chat
 * keys must not share an origin with them), so the site cannot read anything
 * in here and this frame never sees the site's cookie. They talk with
 * postMessage, to and from kryo.to's origin only, and only about the frame:
 *
 * - kryo.to -> chat: `open` a conversation, `focus` (the panel is showing), the
 *   reader's `look`, `signin-start` (the reader pressed Turn on chat), and
 *   `call` actions from the site's call bar.
 * - chat -> kryo.to: its `state`, the `signin-code` to approve, `badge`
 *   counts, `message` (who wrote, never what), the `call` card, and
 *   `navigate` (a profile or a game page, opened by the site).
 *
 * Message text and keys never leave the frame.
 */

/** Framed by kryo.to with `?embed=1`; anywhere else it is the plain web chat. */
export const EMBEDDED = typeof window !== 'undefined' && window.parent !== window && new URLSearchParams(location.search).has('embed')

/** The kryo.to account the site has signed in, from the frame's address. */
export const EXPECTED_USER = EMBEDDED ? new URLSearchParams(location.search).get('u') : null

/**
 * Kryoto Mobile's own page, which frames chat with `?app=1`: the fixed
 * origins a Tauri app is served from (Android, then iOS), and the app's dev
 * server in a dev build. Nothing else is ever trusted as the parent.
 */
const APP_PARENTS = ['http://tauri.localhost', 'https://tauri.localhost', 'tauri://localhost', ...(import.meta.env.DEV ? ['http://localhost:1423'] : [])]

/** Who framed this: kryo.to, or Kryoto Mobile when it says so and really is. */
function parentOrigin(): string {
  const site = new URL(API_BASE).origin
  if (!EMBEDDED || !new URLSearchParams(location.search).has('app')) return site
  const framer = location.ancestorOrigins?.[0]
  return framer && APP_PARENTS.includes(framer) ? framer : site
}

const PARENT = parentOrigin()

export type ToSite =
  | { type: 'state'; state: 'loading' | 'signin' | 'waiting' | 'ready' | 'elsewhere' | 'unavailable' | 'other-account' | 'anonymous'; detail?: string }
  | { type: 'signin-code'; code: string }
  | { type: 'badge'; unread: number; messageRequests: number }
  | { type: 'message'; from: string; peer: string }
  | { type: 'call'; phase: string; peer: string; name: string; since: number | null; muted: boolean; deafened: boolean; reason: string | null; reconnecting: boolean }
  | { type: 'navigate'; path: string }

export type FromSite =
  | { type: 'open'; username: string }
  | { type: 'focus' }
  | { type: 'look'; look: { theme: 'light' | 'dark'; palette: string; radius: string; font: string | null; season: string | null; motion: string | null } }
  | { type: 'signin-start' }
  | { type: 'call'; action: 'accept' | 'decline' | 'hangup' | 'mute' | 'deafen' }

export function toSite(msg: ToSite) {
  if (!EMBEDDED) return
  window.parent.postMessage({ kryoto: 'chat', ...msg }, PARENT)
}

/** Messages from kryo.to (the frame's own parent, on its origin); returns the unsubscribe. */
export function fromSite(handler: (msg: FromSite) => void): () => void {
  if (!EMBEDDED) return () => {}
  const listener = (e: MessageEvent) => {
    if (e.origin !== PARENT || e.source !== window.parent) return
    const d = e.data as { kryoto?: string; type?: string } | null
    if (!d || d.kryoto !== 'chat' || typeof d.type !== 'string') return
    handler(d as unknown as FromSite)
  }
  window.addEventListener('message', listener)
  return () => window.removeEventListener('message', listener)
}

let held = false
let asking: Promise<boolean> | null = null

/**
 * Whether the tab with chat is in a call: a tab that would take chat over
 * asks first, so opening chat somewhere else never cuts a call off.
 */
const channel = typeof BroadcastChannel !== 'undefined' ? new BroadcastChannel('kryoto-chat-engine') : null
let calling = false
export function setEngineInCall(v: boolean) {
  calling = v
}
channel?.addEventListener('message', (e: MessageEvent) => {
  if (e.data === 'ask' && held) channel.postMessage(calling ? 'call' : 'free')
})
export function callElsewhere(): Promise<boolean> {
  const ch = channel
  if (!ch) return Promise.resolve(false)
  return new Promise((resolve) => {
    const answer = (e: MessageEvent) => {
      if (e.data !== 'call' && e.data !== 'free') return
      window.clearTimeout(timer)
      ch.removeEventListener('message', answer)
      resolve(e.data === 'call')
    }
    const timer = window.setTimeout(() => {
      ch.removeEventListener('message', answer)
      resolve(false)
    }, 300)
    ch.addEventListener('message', answer)
    ch.postMessage('ask')
  })
}

/**
 * One chat engine per browser. Two tabs each running one would fight over
 * the gateway connection and over the device's encryption state, so the tab
 * holding this lock runs chat and the others say so.
 *
 * `try` takes it if it is free; `wait` queues for it (kryo.to's tabs in the
 * background, so the next one takes over when the tab with chat closes);
 * `steal` takes it from whichever tab has it, and that tab reloads into
 * "open in another tab". Asked once per page, however often a component asks.
 */
export function holdEngineLock(mode: 'try' | 'wait' | 'steal'): Promise<boolean> {
  if (held) return Promise.resolve(true)
  if (asking && mode !== 'steal') return asking
  const locks = (navigator as Navigator & { locks?: LockManager }).locks
  if (!locks) {
    held = true
    return Promise.resolve(true)
  }
  const options: LockOptions = mode === 'steal' ? { steal: true } : mode === 'try' ? { ifAvailable: true } : {}
  const attempt = new Promise<boolean>((resolve) => {
    let granted = false
    locks
      .request('kryoto-chat-engine', options, (lock) => {
        if (!lock) {
          resolve(false)
          return undefined
        }
        granted = true
        held = true
        resolve(true)
        // Held until the page goes away, or another tab takes it.
        return new Promise<void>(() => {})
      })
      .catch(() => {
        if (!granted) return resolve(false)
        // Taken by another tab: start again as the tab that does not have it.
        held = false
        try {
          sessionStorage.setItem('kryo.chat.moved', '1')
        } catch {
          /* only changes the wording */
        }
        location.reload()
      })
  })
  asking = attempt
  void attempt.then(() => {
    if (asking === attempt) asking = null
  })
  return attempt
}
