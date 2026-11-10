import { useSyncExternalStore } from 'react'
import { isOnline, whenOnline } from '@/lib/online'
import { getVersion } from '@tauri-apps/api/app'
import { relaunch } from '@tauri-apps/plugin-process'
import { check as tauriCheck, type Update } from '@tauri-apps/plugin-updater'
import { errorText, isTauri } from '@/lib/bridge'

/**
 * The in-app updater, the same way Kryoto Forge does it (kryoto-forge
 * src/lib/updates.ts): one store outside the component tree, so the prompt on
 * launch and "Check for updates" in About can never download the same
 * installer twice, and errors are kept rather than swallowed.
 *
 * Where it looks: `latest.json` on the newest GitHub release
 * (`plugins.updater` in tauri.conf.json), written by the release workflow and
 * signed with the key whose public half is in that file.
 */

export type UpdateStatus = 'idle' | 'checking' | 'current' | 'available' | 'downloading' | 'ready' | 'failed'

export type UpdateState = {
  status: UpdateStatus
  current: string
  update: Update | null
  error: string | null
  checkedAt: number | null
  got: number
  total: number | null
}

/** A check must not spin forever on a server that accepts and then stalls. */
const CHECK_TIMEOUT_MS = 20_000

let state: UpdateState = { status: 'idle', current: '', update: null, error: null, checkedAt: null, got: 0, total: null }
const listeners = new Set<() => void>()

function set(patch: Partial<UpdateState>) {
  state = { ...state, ...patch }
  for (const l of listeners) l()
}

const busy = () => state.status === 'checking' || state.status === 'downloading' || state.status === 'ready'

let launchChecked = false

export const updates = {
  snapshot: () => state,

  async loadVersion() {
    if (state.current || !isTauri()) return
    try {
      set({ current: await getVersion() })
    } catch {
      // Only a label.
    }
  },

  async check() {
    if (!isTauri()) {
      set({ status: 'failed', error: 'The browser preview has no updater. Run the app to check for updates.', checkedAt: Date.now() })
      return
    }
    if (busy()) return
    set({ status: 'checking', error: null })
    try {
      const found = await tauriCheck({ timeout: CHECK_TIMEOUT_MS })
      set({ update: found, status: found ? 'available' : 'current', error: null, checkedAt: Date.now(), got: 0, total: null })
    } catch (e) {
      set({ status: 'failed', update: null, error: errorText(e), checkedAt: Date.now() })
    }
  },

  /** Once per launch, quietly: an unreachable server is not worth a dialog. */
  /** Once per run, as soon as there is a connection (offline, it waits for one). */
  async checkOnLaunch() {
    if (launchChecked || !isTauri()) return
    if (!isOnline()) {
      void whenOnline().then(() => updates.checkOnLaunch())
      return
    }
    launchChecked = true
    await updates.check()
  },

  /** Download, install, restart. The restart is the point of no return. */
  async install() {
    const found = state.update
    if (!found || state.status === 'downloading' || state.status === 'ready') return
    set({ status: 'downloading', error: null, got: 0, total: null })
    try {
      await found.downloadAndInstall((event) => {
        if (event.event === 'Started') set({ total: event.data.contentLength ?? null, got: 0 })
        else if (event.event === 'Progress') set({ got: state.got + event.data.chunkLength })
        else if (event.event === 'Finished') set({ status: 'ready' })
      })
      await relaunch()
    } catch (e) {
      set({ status: 'failed', error: errorText(e) })
    }
  },
}

function subscribe(cb: () => void) {
  listeners.add(cb)
  return () => {
    listeners.delete(cb)
  }
}

export function useUpdates(): UpdateState {
  return useSyncExternalStore(subscribe, updates.snapshot, updates.snapshot)
}

/** Download progress as 0..1, or null while the size is unknown. */
export function updateFraction(s: UpdateState): number | null {
  if (!s.total || s.total <= 0) return null
  return Math.min(1, s.got / s.total)
}

/**
 * What a failure means, not just what threw. The same cases as Forge's: a
 * signature from a key this build does not trust is a publishing mistake
 * that retrying cannot fix, not a network problem.
 */
export function explainUpdateError(error: string | null): { headline: string; retryable: boolean } | null {
  if (!error) return null
  const e = error.toLowerCase()
  if (e.includes('different key') || e.includes('signature verification failed') || e.includes('unexpected signature algorithm') || e.includes('invalid encoding in minisign')) {
    return {
      headline: 'This release was signed with a key this app does not trust, so the installer was refused. Get the new version from kryo.to/desktop instead.',
      retryable: false,
    }
  }
  if (e.includes('was not found in the response') || e.includes('platforms` object')) {
    return { headline: 'The new version has no build for this system yet. Nothing is wrong here.', retryable: false }
  }
  if (e.includes('could not fetch a valid release json')) {
    return { headline: 'The update server answered with something that is not a release. Try again in a few minutes.', retryable: true }
  }
  if (e.includes('timed out') || e.includes('timeout') || e.includes('dns') || e.includes('connect') || e.includes('network')) {
    return { headline: 'The update server could not be reached.', retryable: true }
  }
  return null
}
