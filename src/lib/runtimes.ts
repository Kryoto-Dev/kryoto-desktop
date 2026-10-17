import { useEffect, useState } from 'react'
import { call, on } from '@/lib/bridge'

/**
 * Game runtimes (src-tauri/src/runtimes.rs): the Microsoft redistributables
 * most Windows games expect, found missing and installed in one go, with one
 * Windows permission prompt. Windows only: the list is empty elsewhere.
 */

export type Runtime = {
  id: string
  name: string
  group: string
  detail: string
  mb: number
  /** Offered at startup; the rest are in Settings only. */
  recommended: boolean
  installed: boolean
  /** Its installer is downloaded already. */
  cached: boolean
}

export type RuntimeOutcome = { id: string; name: string; ok: boolean; detail: string }

export type RuntimesProgress = {
  phase: 'idle' | 'downloading' | 'permission' | 'installing' | 'done' | 'failed' | 'canceled'
  current: string | null
  step: number
  steps: number
  fraction: number
  message: string | null
  results: RuntimeOutcome[]
  reboot: boolean
}

export const runtimesApi = {
  list: () => call<Runtime[]>('runtimes_list'),
  install: (ids: string[], repair = false) => call<void>('runtimes_install', { ids, repair }),
  progress: () => call<RuntimesProgress | null>('runtimes_progress'),
  cancel: () => call<void>('runtimes_cancel'),
  clearCache: () => call<number>('runtimes_clear_cache'),
}

export const busy = (p: RuntimesProgress | null | undefined) => !!p && (p.phase === 'downloading' || p.phase === 'permission' || p.phase === 'installing')

/** The job now (or the last one), kept current by `runtimes-progress`. */
export function useRuntimesProgress() {
  const [p, setP] = useState<RuntimesProgress | null>(null)
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void runtimesApi.progress().then((v) => !cancelled && setP((cur) => cur ?? v)).catch(() => {})
    void on<RuntimesProgress>('runtimes-progress', (v) => !cancelled && setP(v)).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
  return p
}

/* ── Asking at startup ─────────────────────────────────── */

const ASK_KEY = 'kryo:runtimes-ask'
const LATER_DAYS = 7

/** Whether the start screen may offer the missing runtimes now. */
export function mayAskAtStartup(): boolean {
  try {
    const v = localStorage.getItem(ASK_KEY)
    if (v === 'never') return false
    const later = Number(v)
    return !v || !Number.isFinite(later) || Date.now() > later
  } catch {
    return false
  }
}

/** "Later": not again for a week. */
export function askLater() {
  try {
    localStorage.setItem(ASK_KEY, String(Date.now() + LATER_DAYS * 24 * 3600 * 1000))
  } catch {
    /* asked again next start, which is harmless */
  }
}

export function setAskAtStartup(on: boolean) {
  try {
    if (on) localStorage.removeItem(ASK_KEY)
    else localStorage.setItem(ASK_KEY, 'never')
  } catch {
    /* storage unavailable */
  }
}

export function asksAtStartup(): boolean {
  try {
    return localStorage.getItem(ASK_KEY) !== 'never'
  } catch {
    return true
  }
}
