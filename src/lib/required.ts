import { useEffect, useState } from 'react'
import { call, isTauri, on } from '@/lib/bridge'

/**
 * The oldest Kryoto Desktop kryo.to still allows (src-tauri required.rs): asked
 * at start, every few hours, when the connection comes back, and at once when
 * kryo.to's "Update Kryoto Desktop" page shows in the Store (`required-state`).
 * While `blocked`, the app shows RequiredUpdate instead of the client.
 */
export type RequiredState = {
  blocked: boolean
  min: string | null
  reason: string | null
  latest: string | null
  current: string
  /** kryo.to did not answer: this is the last answer, kept for at most a week. */
  offline: boolean
}

const EVERY_MS = 3 * 60 * 60 * 1000

export function useRequiredUpdate(): RequiredState | null {
  const [state, setState] = useState<RequiredState | null>(null)
  useEffect(() => {
    if (!isTauri()) return
    let alive = true
    const ask = () =>
      void call<RequiredState>('required_check')
        .then((r) => {
          if (alive) setState(r)
        })
        .catch(() => {})
    // What holds since start (the answer kept from last time): a blocked
    // copy shows its screen at once, not once kryo.to has answered.
    void call<RequiredState | null>('required_held')
      .then((r) => {
        if (alive && r) setState((s) => s ?? r)
      })
      .catch(() => {})
    ask()
    const timer = window.setInterval(ask, EVERY_MS)
    window.addEventListener('online', ask)
    const off = on<RequiredState>('required-state', (r) => {
      if (alive) setState(r)
    })
    return () => {
      alive = false
      window.clearInterval(timer)
      window.removeEventListener('online', ask)
      void off.then((f) => f())
    }
  }, [])
  return state
}
