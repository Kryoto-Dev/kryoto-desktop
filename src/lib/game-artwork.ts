import { call, isTauri } from '@/lib/bridge'
import { catalogApiUrl } from '@/lib/endpoint'

export type Artwork = {
  cover: string | null
  hero: string | null
  header?: string | null
  logo?: string | null
  customCover?: string | null
  customHero?: string | null
}
export type ArtMatch = { appid: string; title: string; cover: string | null }

export const isCustomArt = (url: string | null | undefined) => /^kryoto-art:\/\/local\/[a-f0-9]{64}$/.test(url ?? '')
export const titleKey = (title: string) => title.replace(/[\u2122\u00ae\u00a9]/g, '').normalize('NFKC').toLocaleLowerCase('en-US').replace(/[\p{P}\s]+/gu, '')

/**
 * In the app these ask Steam directly (src-tauri steam_art.rs): they are for
 * games kryo.to does not list, whose names and AppIDs stay off Kryoto's
 * servers. Only the browser preview, which cannot reach Steam, uses kryo.to's
 * proxy.
 */
export async function searchArtwork(title: string, signal?: AbortSignal): Promise<ArtMatch[]> {
  if (title.trim().length < 2) return []
  if (isTauri()) return call<ArtMatch[]>('steam_art_search', { q: title.trim() })
  const res = await fetch(await catalogApiUrl(`/api/steam/art/search?q=${encodeURIComponent(title.trim())}`), { signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(7000)]) : AbortSignal.timeout(7000) })
  if (!res.ok) throw new Error('Steam artwork search is unavailable. You can still choose your own pictures.')
  return ((await res.json()) as { results: ArtMatch[] }).results
}

type SteamArt = { capsule: string | null; hero: string | null; header: string | null; logo: string | null }

export async function fetchArtwork(appid: string, signal?: AbortSignal): Promise<Artwork> {
  let art: SteamArt
  if (isTauri()) {
    art = await call<SteamArt>('steam_art', { appid })
  } else {
    const res = await fetch(await catalogApiUrl(`/api/steam/art?appid=${encodeURIComponent(appid)}&library=1`), { signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(7000)]) : AbortSignal.timeout(7000) })
    if (!res.ok) throw new Error('Artwork is unavailable for this game. You can choose your own pictures.')
    art = (await res.json()) as SteamArt
  }
  return { cover: art.capsule ?? art.header, hero: art.hero ?? art.header, header: art.header, logo: art.logo }
}

/** Only an exact title match is automatic; other results need the player's choice. */
export async function automaticArtwork(title: string): Promise<Artwork | null> {
  try {
    const matches = await searchArtwork(title)
    const exact = matches.filter((m) => titleKey(m.title) === titleKey(title))
    const match = exact.length === 1 ? exact[0] : undefined
    return match ? await fetchArtwork(match.appid) : null
  } catch {
    return null
  }
}

export function importArtwork(path: string): Promise<string> {
  return call<string>('art_import', { path })
}

/** Prefer the player's picture in every library layout. */
export const coverFor = (game: Artwork) => game.customCover ?? game.cover
export const heroFor = (game: Artwork) => game.customHero ?? game.hero
