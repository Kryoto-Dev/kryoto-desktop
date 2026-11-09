import { useEffect, useState } from 'react'
import { call, on } from '@/lib/bridge'

/** Mirrors `src-tauri/src/collections.rs`. */

export type CollectionGame = { slug: string; title: string; cover: string | null; nsfw: boolean; sizeBytes: number | null }

export type Collection = {
  id: string
  name: string
  /** Its page on kryo.to. */
  href: string
  role: 'owner' | 'collaborator' | 'following' | 'visitor'
  games: CollectionGame[]
}

export type CollectionUpdate = { id: string; name: string; href: string; added: CollectionGame[] }

export type CollectionsState = {
  collections: Collection[]
  /** The ids installed on this PC, whose new games are asked about. */
  installed: string[]
  updates: CollectionUpdate[]
}

const EMPTY: CollectionsState = { collections: [], installed: [], updates: [] }

export const collections = {
  state: () => call<CollectionsState>('collections_state'),
  /** Queue these games, and watch the collection from now on. How many were queued. */
  install: (id: string, slugs: string[], folder: string | null) => call<number>('collection_install', { id, slugs, folder }),
  /** Not now: these new games are not asked about again. */
  later: (id: string) => call<void>('collection_later', { id }),
  /** Stop watching it. Its games stay. */
  forget: (id: string) => call<void>('collection_forget', { id }),
}

let current = EMPTY
const subs = new Set<(s: CollectionsState) => void>()
let wired = false
function wire() {
  if (wired) return
  wired = true
  const publish = (s: CollectionsState) => {
    current = s
    subs.forEach((fn) => fn(s))
  }
  const load = () => void collections.state().then(publish).catch(() => {})
  load()
  void on<CollectionsState>('collections-state', publish)
  // A game installed or removed changes what is missing and what is news.
  void on('library-changed', load)
}

export function useCollections(): CollectionsState {
  const [state, setState] = useState(current)
  useEffect(() => {
    wire()
    subs.add(setState)
    setState(current)
    return () => {
      subs.delete(setState)
    }
  }, [])
  return state
}

/** Open the install dialog for a collection (`CollectionInstall.tsx`). */
const asks = new Set<(id: string) => void>()
export const askInstall = (id: string) => asks.forEach((fn) => fn(id))
export function onAskInstall(fn: (id: string) => void) {
  asks.add(fn)
  return () => {
    asks.delete(fn)
  }
}

/**
 * What installing these games takes. Games install one at a time and each
 * archive is deleted once it is unpacked, so a drive holds every game and, at
 * the worst moment, the biggest archive beside them (none with HatchZip).
 */
export function installNeeds(games: Pick<CollectionGame, 'sizeBytes'>[], inPlace: boolean) {
  const sizes = games.map((g) => g.sizeBytes ?? 0)
  const download = sizes.reduce((a, b) => a + b, 0)
  return {
    download,
    free: download + (inPlace ? 0 : Math.max(0, ...sizes)),
    /** Games whose size kryo.to does not list: the figures leave them out. */
    unknown: games.filter((g) => !g.sizeBytes).length,
  }
}
