/**
 * How soon the web chat asks kryo.to for the friends list again
 * (`backend.ts`), as a plain function so `pnpm test` runs it.
 *
 * kryo.to answers 403 to an account in Anonymous mode and 404 where the
 * friends feature is not on: the answer will be the same in half a minute, so
 * it is asked again only after half an hour (or sooner when the player does
 * something with friends, or the account is refreshed). Other failures (the
 * site busy, no connection) wait longer each time, from one minute to fifteen.
 */
export const FRIENDS_EVERY_MS = 30_000
export const FRIENDS_REFUSED_MS = 30 * 60_000
const BACKOFF_FIRST_MS = 60_000
const BACKOFF_MAX_MS = 15 * 60_000

/** After a failed ask with this `status` (0: no answer): the wait before the next, and the backoff to remember. */
export function friendsNext(status: number, wait: number): { delay: number; wait: number } {
  if (status === 403 || status === 404) return { delay: FRIENDS_REFUSED_MS, wait: 0 }
  const next = Math.min(wait ? wait * 2 : BACKOFF_FIRST_MS, BACKOFF_MAX_MS)
  return { delay: next, wait: next }
}
