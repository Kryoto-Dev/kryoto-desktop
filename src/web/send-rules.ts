/**
 * What a send does when nothing could be encrypted for the people it is for,
 * though they have chat devices (KB-0256). "They have not turned on chat yet"
 * is said only when they have no device at all, before this. Same rules as
 * native chat (`empty_send` in src-tauri/src/chat/messaging.rs).
 *
 * - `done`: only ourselves with no other device, or a group nobody else in
 *   can be reached right now: nothing to send, and that is fine.
 * - `retry`: a device that has just turned chat on uploads its keys a moment
 *   later, so once more with fresh devices and keys, after `RETRY_MS`.
 * - `starting`: still nothing on the second go: say so, never "chat is off".
 */
export type EmptySend = 'done' | 'retry' | 'starting'

export const RETRY_MS = 1500

export function afterNothingEncrypted(group: boolean, onlyMe: boolean, attempt: number): EmptySend {
  if (group || onlyMe) return 'done'
  return attempt === 0 ? 'retry' : 'starting'
}

export const STILL_STARTING = 'Their chat is still starting. Try again in a moment.'
export const NO_CHAT = 'They have not turned on chat yet.'
