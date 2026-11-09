import { call } from '@/lib/bridge'
import type { LibraryGame } from '@/lib/library'

/**
 * Bridge: Kryoto and other launchers (src-tauri bridge/). Games come in from
 * them and go out to them; their files are only read, unless the player sends
 * games out, and nothing is written until the player has seen the list.
 */

export type Launcher = {
  id: string
  name: string
  /** Its data folder on this PC. */
  dir: string | null
  /** It is open. */
  running: boolean
  /** Its games stay in it, and it can be read while it runs. */
  keeps: boolean
  /** Games can go out to it too. */
  exports: boolean
}

export type BridgeChoice = { slug: string; title: string; developer: string | null; year: number | null; nsfw: boolean }

export type BridgeRow = {
  key: string
  title: string
  playtimeSeconds: number
  lastPlayed: number | null
  executable: string | null
  cover: string | null
  /** `matched` (Kryoto has it), `own` (it does not), `ask` (maybe), `stays` (remains in its launcher), `missing` (not on this PC), `imported` (brought over before). */
  kind: 'matched' | 'own' | 'ask' | 'stays' | 'missing' | 'imported'
  options: BridgeChoice[]
  /** What made the match sure. */
  how: 'steamAppid' | 'executable' | 'folder' | 'title' | null
  /** A game already in the library this one joins, or for `stays`, shares its name with. */
  joins: string | null
  remembered: boolean
}

export type BridgeScan = { dir: string | null; running: boolean; catalog: boolean; rows: BridgeRow[] }

/** What a row becomes: a kryo.to game, or the player's own (`slug: null`). */
export type BridgeDecision = { key: string; slug: string | null }

export type BridgeImported = { linked: number; own: number; joined: number; seconds: number; /** Save versions are on: the games' saves are being kept now. */ saves: boolean }

export type ExportGame = { id: string; title: string; cover: string | null; nsfw: boolean; /** Already there, from an earlier send. */ sent: boolean }

export type ExportList = {
  users: { id: string; name: string }[]
  games: ExportGame[]
  /** Something the player should know first. */
  note: string | null
  /** How many games an undo would take back. */
  undoable: number
}

export type Sent = { added: number; updated: number; pictures: number; /** The copy of the launcher's list from before. */ backup: string | null }

export type MergeOffer = { id: string; title: string; slug: string; kryoTitle: string }

export const bridge = {
  launchers: () => call<Launcher[]>('bridge_launchers'),
  detect: (launcherId: string, dir: string | null) => call<Launcher>('bridge_detect', { launcherId, dir }),
  scan: (launcherId: string, dir: string | null) => call<BridgeScan>('bridge_scan', { launcherId, dir }),
  import: (launcherId: string, dir: string | null, decisions: BridgeDecision[]) => call<BridgeImported>('bridge_import', { launcherId, dir, decisions }),
  exportList: (launcherId: string, dir: string | null) => call<ExportList>('bridge_export_list', { launcherId, dir }),
  export: (launcherId: string, dir: string | null, user: string, ids: string[], adultArt: boolean) => call<Sent>('bridge_export', { launcherId, dir, user, ids, adultArt }),
  undo: (launcherId: string, dir: string | null) => call<number>('bridge_export_undo', { launcherId, dir }),
}

/** A launcher that keeps a library game, as the player reads it. */
export function launcherName(id: string | null | undefined): string | null {
  if (!id) return null
  return id === 'steam' ? 'Steam' : id.charAt(0).toUpperCase() + id.slice(1)
}

/** The player's own games kryo.to now lists, and the yes or no to linking each. */
export const merges = {
  offers: () => call<MergeOffer[]>('library_merge_offers'),
  answer: (id: string, slug: string, yes: boolean) => call<LibraryGame>('library_merge_answer', { id, slug, yes }),
}

/** Hours from before a game was linked: sent once; marked sent when kryo.to answers (src-tauri lib.rs `store_carry_done`). */
export const carry = {
  send: (id: string, slug: string, seconds: number, key: string) => call<void>('store_report_carry', { id, slug, seconds, key }),
}
