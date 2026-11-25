/**
 * The player's download order (suggestion #110): which source a build comes
 * from first. Mirror hosts by the name the resolvers give them (src-tauri
 * resolvers/mod.rs `known_hosts`), and `kryoto` for our own copy. An empty
 * order is kryo.to's: our copy first, then its mirrors as it lists them.
 * Pure, for `pnpm test`; the core hands a failed download on in the same
 * order (downloads.rs `next_source`).
 */

/** Our own copy, in the order. */
export const OURS = 'kryoto'

/** A source's name in the order. */
function nameOf(source: { kind: string; host?: string }): string {
  return source.kind === 'ours' ? OURS : (source.host ?? '')
}

/** Where `name` stands in the order: its place, or after every name it has. */
function rank(order: string[], name: string): number {
  const i = order.findIndex((o) => o.toLowerCase() === name.toLowerCase())
  return i < 0 ? order.length : i
}

/**
 * A release's sources, the player's picks first, in their order; the rest
 * keep the order they came in. The first is what a download button gets.
 */
export function orderSources<S extends { kind: string; host?: string }>(sources: S[], order: string[]): S[] {
  if (!order.length) return sources
  return sources
    .map((s, i) => ({ s, i, r: rank(order, nameOf(s)) }))
    .sort((a, b) => a.r - b.r || a.i - b.i)
    .map((x) => x.s)
}

/**
 * The whole list Settings shows: the saved order, our copy after it when the
 * order does not place it (first when nothing was set, as today), then every
 * other host the app knows.
 */
export function orderList(order: string[], hosts: string[]): string[] {
  const list = [...order]
  for (const name of [OURS, ...hosts]) {
    if (rank(list, name) === list.length) list.push(name)
  }
  return list
}

/** `list` with the entry at `from` moved up (-1) or down (+1); unchanged at either end. */
export function moveIn(list: string[], from: number, by: -1 | 1): string[] {
  const to = from + by
  if (from < 0 || from >= list.length || to < 0 || to >= list.length) return list
  const next = [...list]
  next.splice(to, 0, ...next.splice(from, 1))
  return next
}
