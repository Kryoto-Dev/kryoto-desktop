/**
 * kryo.to's answer to a request the Store's page sent with the player's
 * session (`store-friend-done`, lib/friends.ts `viaStore`): the data it sent
 * back, or its own words when it refused, so the player reads why. Pure, for
 * `pnpm test` and the browser preview.
 */
export type StoreDone = { id: string; ok: boolean; status: number; data?: Record<string, unknown> | null }

export function storeAnswer(r: Omit<StoreDone, 'id'>): Record<string, unknown> {
  const data = r.data ?? {}
  if (r.ok) return data
  if (typeof data.error === 'string' && data.error.trim()) throw new Error(data.error)
  throw new Error(r.status === 0 ? 'Could not reach kryo.to.' : `kryo.to answered ${r.status}.`)
}
