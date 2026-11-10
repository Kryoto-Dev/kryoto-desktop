import { useEffect, useState } from 'react'
import { call, on } from '@/lib/bridge'

/** Mirrors `src-tauri/src/patchbay/mod.rs`. */

export type PatchbaySlot = 'saves' | 'achievements' | 'real-debrid' | 'torbox'

export type PatchbayStatus = {
  saves: { endpoint: string | null; token: boolean }
  achievements: { endpoint: string | null; token: boolean }
  debrid: 'real-debrid' | 'torbox' | null
  /** A Real-Debrid token is kept. */
  debridToken: boolean
  torboxToken: boolean
}

export const patchbay = {
  status: () => call<PatchbayStatus>('patchbay_status'),
  /** Keep a token in the OS secure store, or forget it with `null`. It never comes back out. */
  setToken: (slot: PatchbaySlot, token: string | null) => call<PatchbayStatus>('patchbay_set_token', { slot, token }),
  /** A sentence on what the test found; throws what went wrong. */
  test: (slot: PatchbaySlot) => call<string>('patchbay_test', { slot }),
}

export function usePatchbay() {
  const [status, setStatus] = useState<PatchbayStatus | null>(null)
  useEffect(() => {
    let cancelled = false
    const load = () => void patchbay.status().then((s) => !cancelled && setStatus(s)).catch(() => {})
    load()
    const off = on('patchbay', load)
    return () => {
      cancelled = true
      void off.then((f) => f())
    }
  }, [])
  return [status, setStatus] as const
}
