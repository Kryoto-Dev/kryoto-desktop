import { useSyncExternalStore } from 'react'
import { isOnline, whenOnline } from '@/lib/online'
import { getVersion } from '@tauri-apps/api/app'
import { relaunch } from '@tauri-apps/plugin-process'
import { check as tauriCheck, Update } from '@tauri-apps/plugin-updater'
import { call, errorText, isTauri } from '@/lib/bridge'
import { IS_PREVIEW_BUILD } from '@/lib/endpoint'

/**
 * The in-app updater, the same way Kryoto Forge does it (kryoto-forge
 * src/lib/updates.ts): one store outside the component tree, so the prompt on
 * launch and "Check for updates" in About can never download the same
 * installer twice, and errors are kept rather than swallowed.
 *
 * Where it looks: `latest.json` on the newest GitHub release
 * (`plugins.updater` in tauri.conf.json), written by the release workflow and
 * signed with the key whose public half is in that file. When GitHub cannot be
 * reached (a network that filters it), kryo.to's copy of the same release,
 * over this app's own way past such networks (src-tauri update_mirror.rs).
 * When nothing works, the player is told so in plain words with the direct
 * download (`DIRECT_DOWNLOAD`), never left with nothing.
 */

/** The newest release's page, where the installer can be downloaded by hand. */
export const DIRECT_DOWNLOAD = IS_PREVIEW_BUILD
  ? 'https://github.com/Kryoto-Dev/kryoto-desktop/releases/tag/preview'
  : 'https://github.com/Kryoto-Dev/kryoto-desktop/releases/latest'

export type UpdateStatus = 'idle' | 'checking' | 'current' | 'available' | 'downloading' | 'ready' | 'failed'

export type UpdateState = {
  status: UpdateStatus
  current: string
  update: Update | null
  error: string | null
  checkedAt: number | null
  got: number
  total: number | null
  /** The last check reached neither GitHub nor kryo.to. */
  unreachable: boolean
}

/** A check must not spin forever on a server that accepts and then stalls. */
const CHECK_TIMEOUT_MS = 20_000

let state: UpdateState = { status: 'idle', current: '', update: null, error: null, checkedAt: null, got: 0, total: null, unreachable: false }
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
    set({ status: 'checking', error: null, unreachable: false })
    try {
      const found = await tauriCheck({ timeout: CHECK_TIMEOUT_MS })
      set({ update: found, status: found ? 'available' : 'current', error: null, checkedAt: Date.now(), got: 0, total: null })
    } catch (e) {
      const first = errorText(e)
      // GitHub out of reach: kryo.to's copy, the app's own way past filters.
      if (!unreachableError(first)) {
        set({ status: 'failed', update: null, error: first, checkedAt: Date.now() })
        return
      }
      try {
        const found = await checkThroughKryoto()
        set({ update: found, status: found ? 'available' : 'current', error: null, checkedAt: Date.now(), got: 0, total: null })
      } catch (e2) {
        set({ status: 'failed', update: null, error: `${first} (kryo.to: ${errorText(e2)})`, checkedAt: Date.now(), unreachable: true })
      }
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
    const run = (u: Update) =>
      u.downloadAndInstall((event) => {
        if (event.event === 'Started') set({ total: event.data.contentLength ?? null, got: 0 })
        else if (event.event === 'Progress') set({ got: state.got + event.data.chunkLength })
        else if (event.event === 'Finished') set({ status: 'ready' })
      })
    try {
      await run(found)
      await relaunch()
    } catch (e) {
      const first = errorText(e)
      // The file could not be fetched from GitHub: the same release through kryo.to.
      // Only while the file was coming: an installer Windows refused (status
      // 'ready') is not something another copy of it gets past.
      if (unreachableError(first) && (state.status as UpdateStatus) === 'downloading') {
        try {
          const again = await checkThroughKryoto()
          if (again) {
            set({ update: again, got: 0, total: null })
            await run(again)
            await relaunch()
            return
          }
        } catch (e2) {
          set({ status: 'failed', error: `${first} (kryo.to: ${errorText(e2)})`, unreachable: true })
          return
        }
      }
      set({ status: 'failed', error: first })
    }
  },
}

/** The update as kryo.to's copy of the release offers it (src-tauri update_mirror.rs). */
async function checkThroughKryoto(): Promise<Update | null> {
  const found = await call<ConstructorParameters<typeof Update>[0] | null>('update_check_mirror')
  return found ? new Update(found) : null
}

/** A failure to reach the server or fetch the file, which another way in might get past. */
export function unreachableError(error: string): boolean {
  const e = error.toLowerCase()
  return [
    'timed out',
    'timeout',
    'dns',
    'connect',
    'network',
    'error sending request',
    'os error 10054',
    'forcibly closed',
    'connection reset',
    'certificate',
    'unknownissuer',
    'handshake',
    'proxy',
  ].some((w) => e.includes(w))
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
export function explainUpdateError(error: string | null): { headline: string; retryable: boolean; manual?: boolean } | null {
  if (!error) return null
  const e = error.toLowerCase()
  // Windows refused to run the downloaded installer: Smart App Control, or a
  // policy set on the PC (application control). Nothing here gets past it.
  if (e.includes('os error 4551') || e.includes('application control')) {
    return {
      headline:
        'Windows blocked the installer (Smart App Control, or a policy on this PC). Download the new version yourself and run it. If Windows blocks that too, whoever manages this PC can allow it.',
      retryable: false,
      manual: true,
    }
  }
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
  if (unreachableError(error)) {
    return {
      headline: 'Neither GitHub nor kryo.to could be reached from this network, so the update could not download. Download the new version yourself.',
      retryable: true,
      manual: true,
    }
  }
  // Anything else, Windows' own error numbers among them ("The resource id ...
  // is invalid"): said plainly, with the way that always works.
  return { headline: 'The update could not be installed. Download the new version yourself and run it.', retryable: true, manual: true }
}
