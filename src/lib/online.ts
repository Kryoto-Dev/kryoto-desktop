import { useSyncExternalStore } from 'react'

/**
 * Whether this PC has a connection, kept current.
 *
 * Offline, the client still does everything that does not need kryo.to: the
 * Library, playing, installed games' pages and art (cached on disk), Settings
 * and the downloads already on disk. What needs the network says so once,
 * calmly, instead of failing in a dozen places.
 *
 * The system's word is checked before it is believed. WebKitGTK takes
 * `navigator.onLine` from GLib's network monitor, which says offline on Linux
 * systems without NetworkManager (WSL, minimal installs, iwd or
 * systemd-networkd setups) while the network works: the Store refused to load
 * and the bar said OFFLINE. So "offline" is confirmed with a real request, and
 * re-checked every half minute until a request gets through.
 */

const PROBE = 'https://kryo.to/favicon.ico'
const RECHECK_MS = 30_000

let online = typeof navigator === 'undefined' || navigator.onLine
const listeners = new Set<() => void>()
let timer: number | undefined

function set(next: boolean) {
  if (next === online) return
  online = next
  for (const l of listeners) l()
}

async function reachable(): Promise<boolean> {
  try {
    // Opaque is enough: a response of any kind means the network is there.
    await fetch(PROBE, { mode: 'no-cors', cache: 'no-store' })
    return true
  } catch {
    return false
  }
}

async function check() {
  window.clearTimeout(timer)
  if (navigator.onLine) return set(true)
  const ok = await reachable()
  set(ok)
  if (!ok) timer = window.setTimeout(() => void check(), RECHECK_MS)
}

if (typeof window !== 'undefined') {
  window.addEventListener('online', () => void check())
  window.addEventListener('offline', () => void check())
  if (!online) void check()
}

export function isOnline() {
  return online
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/** Called with the new state whenever it changes (the pet window, outside React). Returns the way to stop. */
export function onOnlineChange(listener: (online: boolean) => void): () => void {
  return subscribe(() => listener(online))
}

export function useOnline() {
  return useSyncExternalStore(subscribe, isOnline, () => true)
}

/** Resolves once there is a connection (at once when there is one). */
export function whenOnline(): Promise<void> {
  if (online) return Promise.resolve()
  return new Promise((resolve) => {
    const stop = subscribe(() => {
      if (online) {
        stop()
        resolve()
      }
    })
  })
}
