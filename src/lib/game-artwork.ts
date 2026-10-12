import { call } from '@/lib/bridge'
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

export async function searchArtwork(title: string, signal?: AbortSignal): Promise<ArtMatch[]> {
  if (title.trim().length < 2) return []
  const res = await fetch(await catalogApiUrl(`/api/steam/art/search?q=${encodeURIComponent(title.trim())}`), { signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(7000)]) : AbortSignal.timeout(7000) })
  if (!res.ok) throw new Error('Steam artwork search is unavailable. You can still choose your own pictures.')
  return ((await res.json()) as { results: ArtMatch[] }).results
}

export async function fetchArtwork(appid: string, signal?: AbortSignal): Promise<Artwork> {
  const res = await fetch(await catalogApiUrl(`/api/steam/art?appid=${encodeURIComponent(appid)}&library=1`), { signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(7000)]) : AbortSignal.timeout(7000) })
  if (!res.ok) throw new Error('Artwork is unavailable for this game. You can choose your own pictures.')
  const art = (await res.json()) as { capsule: string | null; hero: string | null; header: string | null; logo: string | null }
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
