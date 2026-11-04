import { call, isTauri } from '@/lib/bridge'

/**
 * System notifications - the ones Windows, Linux and macOS show in the corner, like a
 * browser's. Used for kryo.to's own notifications as they arrive and for the
 * app's news (a game ready to play), so nothing is only in the app.
 *
 * Sent by the native side (`os_notify`), never through a plugin that patches
 * `window.Notification`: the Store's pages must keep the browser's own, or
 * Cloudflare's download check fails (see `system.rs`).
 */
export async function notify(title: string, body?: string) {
  if (!isTauri()) return
  try {
    await call('os_notify', { title, body: body ?? null })
  } catch {
    /* the system said no; the in-app list still has it */
  }
}
