import type { SettingsSection } from '@/settings/SettingsPage'

/**
 * The client's one history, behind its Back and Forward.
 *
 * Every place in the client is an entry: the Library, a game (installed or
 * only on one of your kryo.to lists), Downloads, the
 * Community page, Settings, and each Store page, which carries its own
 * address. Back always goes to the entry before, wherever that was, so a
 * profile opened from the Community page goes back to the Community page and
 * not to whatever the Store showed before it. The Store's web view is only
 * ever moved to the address of the entry being shown (see `Shell`).
 *
 * The Store's own page reports each change of address with how it happened
 * (`BROWSER_STATE_SCRIPT` in lib.rs), which is how a click inside kryo.to
 * becomes a new entry while a redirect, or a page tidying its own address
 * with `replaceState`, does not add a step.
 */

export type View =
  | { kind: 'web'; url: string }
  | { kind: 'home' }
  | { kind: 'game'; id: string }
  /** A kryo.to game on one of your lists that is not on this PC. */
  | { kind: 'catalog'; slug: string }
  | { kind: 'downloads' }
  | { kind: 'friends' }
  | { kind: 'settings'; section: SettingsSection }

export type History = { stack: View[]; index: number }

/** How the Store's page moved: a full load, `pushState`, `replaceState`, or its own back/forward. */
export type WebNav = 'load' | 'push' | 'replace' | 'pop'

const LIMIT = 100

/** Two addresses are the same page when only a trailing slash or `#fragment` differs. */
export function sameUrl(a: string, b: string) {
  const norm = (u: string) => u.replace(/#.*$/, '').replace(/\/+(\?|$)/, '$1')
  return norm(a) === norm(b)
}

export function sameView(a: View, b: View) {
  if (a.kind !== b.kind) return false
  switch (a.kind) {
    case 'web':
      return sameUrl(a.url, (b as typeof a).url)
    case 'game':
      return a.id === (b as typeof a).id
    case 'catalog':
      return a.slug === (b as typeof a).slug
    case 'settings':
      return a.section === (b as typeof a).section
    default:
      return true
  }
}

export function start(view: View): History {
  return { stack: [view], index: 0 }
}

export function current(h: History): View {
  return h.stack[h.index] ?? { kind: 'home' }
}

/** Go somewhere new: it becomes the next entry, and anything forward of here is dropped. */
export function go(h: History, next: View): History {
  if (sameView(current(h), next)) return h
  const stack = [...h.stack.slice(0, h.index + 1), next].slice(-LIMIT)
  return { stack, index: stack.length - 1 }
}

/** Back (-1) or forward (+1) one entry. */
export function step(h: History, by: -1 | 1): History {
  const index = Math.min(h.stack.length - 1, Math.max(0, h.index + by))
  return index === h.index ? h : { ...h, index }
}

function replaceCurrent(h: History, view: View): History {
  const stack = h.stack.slice()
  stack[h.index] = view
  return { ...h, stack }
}

/**
 * The Store's page is now at `url`. `expected` means the client sent it
 * somewhere and this is where it landed, so a redirect replaces the entry
 * instead of adding one. Only counts while a Store entry is showing: the
 * Store is also borrowed by Settings, and a page it loads in the background
 * is nobody's step.
 */
export function webMoved(h: History, url: string, nav: WebNav, expected: boolean): History {
  const here = current(h)
  if (here.kind !== 'web' || sameUrl(here.url, url)) return h
  if (expected || nav === 'replace') return replaceCurrent(h, { kind: 'web', url })
  const prev = h.stack[h.index - 1]
  const next = h.stack[h.index + 1]
  // The page went back or forward itself (a modal closing with history.back()).
  if (nav === 'pop' && prev?.kind === 'web' && sameUrl(prev.url, url)) return { ...h, index: h.index - 1 }
  if (nav === 'pop' && next?.kind === 'web' && sameUrl(next.url, url)) return { ...h, index: h.index + 1 }
  return go(h, { kind: 'web', url })
}
