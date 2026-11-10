import { useEffect, useState } from 'react'
import { call, isTauri, on } from '@/lib/bridge'

/**
 * Whether Kryoto is answering, from the native watcher (src-tauri health.rs),
 * which has already sat out a blip: nothing here reacts to one failed request.
 */

export type ServiceState = 'ok' | 'partial' | 'down' | 'unreachable'

export type ServiceHealth = {
  state: ServiceState
  /** What is affected, in the player's words, for `partial`. */
  parts: string[]
  /** The last request to the site got an answer. */
  answering: boolean
  /** The status page answered last time it was asked. */
  statusUp: boolean
  statusUrl: string
}

export const STATUS_URL = 'https://status.kryo.to'

const CALM: ServiceHealth = { state: 'ok', parts: [], answering: true, statusUp: true, statusUrl: STATUS_URL }

export const isOutage = (h: ServiceHealth) => h.state === 'down' || h.state === 'unreachable'

/** Something failed, or Try again was pressed: look now, not on the timer. */
export function pokeHealth() {
  if (isTauri()) void call('health_poke').catch(() => {})
}

export function useServiceHealth(): ServiceHealth {
  const [health, setHealth] = useState(CALM)
  useEffect(() => {
    if (!isTauri()) return
    let cancelled = false
    void call<ServiceHealth | null>('health_get')
      .then((h) => !cancelled && h && setHealth(h))
      .catch(() => {})
    const stop = on<ServiceHealth>('service-health', setHealth)
    // Back at the window after a while: hidden, the watcher only looks now and then.
    let lookedAt = Date.now()
    const back = () => {
      if (Date.now() - lookedAt < 60_000) return
      lookedAt = Date.now()
      pokeHealth()
    }
    window.addEventListener('focus', back)
    return () => {
      cancelled = true
      window.removeEventListener('focus', back)
      void stop.then((f) => f())
    }
  }, [])
  return health
}

/** "Chat", "Chat and Downloads", "Chat, Downloads and Sign-in". */
export function namedParts(parts: string[]): string {
  if (parts.length < 2) return parts[0] ?? 'Part of Kryoto'
  return `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`
}
