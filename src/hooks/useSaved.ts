import { useEffect, useState } from 'react'
import { call, isTauri, on } from '@/lib/bridge'
import { fetchCatalogGame, type LibraryGame } from '@/lib/library'

/**
 * The signed-in account's kryo.to library - the games marked Playing, Plan to
 * Play, Favorite and so on, on the site - as the Store's page reports it.
 * Shown as collections in the client's sidebar, and set from a game's page.
 */

export type SavedStatus = 'playing' | 'plan' | 'completed' | 'onhold' | 'dropped' | 'favorite' | 'notinterested'
export type SavedEntry = { slug: string; status: SavedStatus; title: string; cover: string; updatedAt: string }

export const STATUS_LABEL: Record<SavedStatus, string> = {
  playing: 'Playing',
  plan: 'Plan to Play',
  completed: 'Completed',
  onhold: 'On Hold',
  dropped: 'Dropped',
  favorite: 'Favorite',
  notinterested: 'Not interested',
}
export const STATUSES = Object.keys(STATUS_LABEL) as SavedStatus[]

/** "Not interested" keeps a game out of rolls and picks; it is not something they play, so no list or count shows it as played. */
export const isPlayStatus = (status: SavedStatus) => status !== 'notinterested'

const PREVIEW: SavedEntry[] = [
  { slug: 'captain-hardcore', status: 'playing', title: 'Captain Hardcore', cover: '', updatedAt: '' },
  { slug: 'hades-ii', status: 'plan', title: 'Hades II', cover: 'https://cdn.cloudflare.steamstatic.com/steam/apps/1145350/library_600x900.jpg', updatedAt: '' },
  { slug: 'celeste', status: 'favorite', title: 'Celeste', cover: '', updatedAt: '' },
]

let current: SavedEntry[] = isTauri() ? [] : PREVIEW
const subs = new Set<(e: SavedEntry[]) => void>()
let hiddenNow: string[] = []
const hiddenSubs = new Set<(slugs: string[]) => void>()
let wired = false
function wire() {
  if (wired) return
  wired = true
  void on<{ entries?: SavedEntry[]; hidden?: string[] }>('saved-state', (p) => {
    current = p.entries ?? []
    subs.forEach((fn) => fn(current))
    hiddenNow = p.hidden ?? []
    hiddenSubs.forEach((fn) => fn(hiddenNow))
  })
}

/** The games the account keeps to itself, on this PC or not. */
export function useHiddenSlugs(): string[] {
  const [slugs, setSlugs] = useState(hiddenNow)
  useEffect(() => {
    wire()
    hiddenSubs.add(setSlugs)
    setSlugs(hiddenNow)
    return () => {
      hiddenSubs.delete(setSlugs)
    }
  }, [])
  return slugs
}

export function useSaved(): SavedEntry[] {
  const [entries, setEntries] = useState(current)
  useEffect(() => {
    wire()
    subs.add(setEntries)
    setEntries(current)
    return () => {
      subs.delete(setEntries)
    }
  }, [])
  return entries
}

/** Change a game's status on kryo.to (`null` takes it out of the library). */
export function setSavedStatus(
  slug: string,
  status: SavedStatus | null,
  game?: Pick<LibraryGame, 'title' | 'cover'>,
) {
  if (!isTauri()) {
    current = status
      ? [...current.filter((e) => e.slug !== slug), {
          slug,
          status,
          title: game?.title || slug,
          cover: game?.cover || '',
          updatedAt: '',
        }]
      : current.filter((e) => e.slug !== slug)
    subs.forEach((fn) => fn(current))
    return Promise.resolve()
  }
  return call<void>('store_set_status', {
    slug,
    status,
    title: game?.title ?? '',
    cover: game?.cover ?? '',
  })
}

/* Saved entries carry a cover but not whether the game is adult, so their
   art stays blurred until the catalog has said. One lookup per game, kept
   for the session. */
const nsfwBySlug = new Map<string, boolean>()
const pending = new Set<string>()

export function useAdultFlags(slugs: string[]) {
  const [, bump] = useState(0)
  useEffect(() => {
    const todo = slugs.filter((s) => !nsfwBySlug.has(s) && !pending.has(s)).slice(0, 40)
    if (!todo.length) return
    let cancelled = false
    todo.forEach((s) => pending.add(s))
    void Promise.all(
      todo.map((s) =>
        fetchCatalogGame(s)
          .then((g) => nsfwBySlug.set(s, !!g.nsfw))
          .catch(() => {})
          .finally(() => pending.delete(s)),
      ),
    ).then(() => !cancelled && bump((n) => n + 1))
    return () => {
      cancelled = true
    }
  }, [slugs])
  /** Unknown counts as adult: blurred until proven otherwise. */
  return (slug: string) => nsfwBySlug.get(slug) ?? true
}
