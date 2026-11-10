import { call, errorText, on } from '@/lib/bridge'
import { logWarn } from '@/lib/log'
import { catalogApiUrl } from '@/lib/endpoint'
import { automaticArtwork, coverFor } from '@/lib/game-artwork'

/**
 * The Library: games on this PC and how each one starts.
 *
 * Mirrors `src-tauri/src/library.rs`. What a Play press actually runs is
 * decided in Rust (`launch.rs`); this side picks, shows and saves.
 */

export type LaunchEntry = {
  executable: string
  arguments: string
  workingdir: string
  description: string
  oslist: string
  /** Steam's kind: "default", "vr", "config"... */
  type?: string
}

/** A game the folder scan found (`library_find_kryoto_games`). */
export type FoundGame = {
  title: string
  installDir: string
  cover: string | null
  nsfw: boolean
  playtimeSeconds: number
  /** Already in the library: this is where it went. */
  moved?: boolean
}

export type LibraryGame = {
  id: string
  title: string
  /** The Steam branch its kryo.to listing builds from: `public`, or a beta such as GoreBox27's `gb27`. */
  steamBranch?: string | null
  slug: string | null
  cover: string | null
  hero: string | null
  customCover?: string | null
  customHero?: string | null
  /** The transparent title logo, when Steam has one. */
  logo?: string | null
  /** The wide store header, for rows and cards. */
  header?: string | null
  installDir: string
  executable: string
  defaultArgs: string
  entries: LaunchEntry[]
  source: string | null
  preferredEntry: number | null
  /** Play asks how to start the game each time. */
  askEachTime: boolean
  launchOptions: string
  compatTool: string | null
  applyOverrides: boolean
  /** Windows: Play starts it as administrator. */
  runAsAdmin?: boolean
  /** The player changed its files: Kryoto stops offering updates and asking to repair it. */
  keepFiles?: boolean
  playtimeSeconds: number
  lastPlayed: number | null
  addedAt: number
  version: string | null
  /** The SHA-256 of the archive it came from, when kryo.to listed one (src-tauri library.rs `build_sha`). */
  buildSha?: string | null
  /** A build picked under Versions: while it is installed, no update is offered. */
  pinnedVersion?: string | null
  short: string | null
  developer: string | null
  /** kryo.to marks it an adult game; its art is blurred unless Settings says otherwise. */
  nsfw: boolean
  /** Add-ons put into it, with the files each wrote (for Undo). */
  addons?: InstalledAddon[]
  /** Kryoto Online set up on this PC. */
  online?: { version: string; added: string[]; saved: [string, string][]; appliedAt: number } | null
  /** This game's in-game name, over Settings'; null follows Settings. */
  playerNameMode?: import('@/lib/settings').PlayerNameMode | null
  playerName?: string
  /** This game's language, over Settings': null follows Settings, empty leaves the build's own. */
  gameLanguage?: string | null
  /** Hours found in the game's folder when it was installed again, not counted yet. */
  foundPlaytime?: number | null
  /** The answer to "restore your playtime?": `never`, `restored`, or none yet. */
  playtimePrompt?: string | null
  /** kryo.to's VR flag: a VR game, a game that also plays in VR, or neither. */
  vr?: VrKind | null
  /** Another launcher's game it came from: `hydra:<shop>:<objectId>`, `steam:<appid>`. */
  origin?: string | null
  /** The launcher that keeps this game (`steam`): Play goes through it, and its files are that launcher's. */
  launcher?: string | null
  /** Kept by src-tauri discord_rpc/: use `discordGames` in lib/discord.ts. */
  discordShow?: boolean | null
  /** The player keeps it to themselves: in the Hidden group, seen by nobody else. Use `library.setHidden`. */
  hidden?: boolean
  /** Play time from before it was linked to kryo.to, on its way to the account once each (src-tauri bridge/). */
  carrySends?: { key: string; seconds: number }[]
}

export type VrKind = 'vr_supported' | 'vr_only'

/** kryo.to's VR flag, null-safe: anything else is no VR at all. */
export function vrKind(v: unknown): VrKind | null {
  return v === 'vr_supported' || v === 'vr_only' ? v : null
}

/** One VR runtime on this PC (src-tauri vr.rs). */
export type VrRuntime = { id: string; name: string; manifest: string | null; starter: string | null; active: boolean }
export type VrStatus = { runtimes: VrRuntime[]; steamvrInstalled: boolean; steamvrRunning: boolean; experimental: boolean }
export const vrApi = { status: () => call<VrStatus>('vr_status') }

/** Play time Play can give back (src-tauri library.rs `playtime_offer`). */
export type PlaytimeOffer = { seconds: number; source: 'folder' | 'account' | 'both' }

export type InstalledAddon = { label: string; file: string; files: string[]; /** Game files it replaced, kept for Undo (missing: applied before 0.3.0). */ replaced?: string[]; installedAt: number }

/** One of a game's add-ons on kryo.to (`/api/games/<slug>/downloads`). */
export type KryoAddon = {
  id: number | string
  label: string | null
  note: string | null
  source: string | null
  version: string | null
  download_size: string | null
  links: { name: string | null }[]
  /** `repair`: a fix (Online-Fix's generic fix) applied under Properties > Repair, with undo. */
  kind?: 'addon' | 'repair'
  /**
   * kryo.to's word on whether to offer it for this game: `false` for the
   * Online add-on on a game Steam lists no online play for, or whose release
   * already brings its own. Absent from older answers.
   */
  offered?: boolean
}

/**
 * Marks a "latest" that is the installed build number uploaded again: the file
 * on kryo.to was replaced under the same number, or a new upload came with no
 * number at all. Compared as a version, it differs from the one installed, so
 * every place that offers updates offers it.
 */
export const NEW_UPLOAD = ' (new upload)'

/** The build kryo.to offers this installed game now, given its current version: that, or the same one marked {@link NEW_UPLOAD}. */
export async function latestFor(game: Pick<LibraryGame, 'slug' | 'version' | 'buildSha'>, version: string | null): Promise<string | null> {
  if (!game.slug || !game.buildSha || (version && version !== (game.version ?? ''))) return version
  // The same number (or none): only the file says whether it is the same build.
  // Read even when the links are withheld: the hashes still come.
  const res = await fetch(await catalogApiUrl(`/api/games/${encodeURIComponent(game.slug)}/downloads`)).catch(() => null)
  const json = res ? ((await res.json().catch(() => null)) as { hashes?: { sha256?: string }[] } | null) : null
  const hashes = (json?.hashes ?? []).map((h) => h.sha256?.toLowerCase()).filter((h): h is string => !!h && h.length === 64)
  if (!hashes.length || hashes.includes(game.buildSha.toLowerCase())) return version
  return `${version || game.version || 'This build'}${NEW_UPLOAD}`
}

/** A game kryo.to can say has an update: on a known build number, or a known file. */
export const knowsBuild = (g: Pick<LibraryGame, 'version' | 'buildSha'>) => !!g.version || !!g.buildSha

export async function fetchAddons(slug: string): Promise<KryoAddon[]> {
  const res = await fetch(await catalogApiUrl(`/api/games/${encodeURIComponent(slug)}/downloads`))
  if (!res.ok) return []
  const json = (await res.json()) as { addons?: KryoAddon[] }
  return json.addons ?? []
}

/** Where a release can be downloaded from, as Kryoto Desktop sees it. */
export type ReleaseSource =
  /** Our own copy: through the Store's download sheet (it passes kryo.to's check). */
  | { kind: 'ours' }
  /** A mirror Kryoto downloads itself. `page` hosts open their page and may ask for a check. */
  | { kind: 'mirror'; host: string; url: string; page: boolean }

/** One full release of a game (`releases` in `/api/games/<slug>/downloads`). */
export type KryoRelease = {
  id: string
  version: string
  label: string | null
  source: string | null
  downloadSize: string | null
  createdAt: string | null
  /** The current release: what Install and updates get. */
  primary: boolean
  /** We no longer keep our own copy: mirrors only. */
  archived: boolean
  sources: ReleaseSource[]
  /** Mirrors Kryoto can't fetch itself (MEGA, torrents): open in the browser. */
  elsewhere: { host: string; url: string }[]
}

type RawLink = { host?: string; url?: string }
type RawRelease = {
  id: string | number
  version?: string | null
  label?: string | null
  source?: string | null
  download_size?: string | null
  created_at?: string | null
  primary?: boolean
  archived?: boolean
  links?: RawLink[]
}

/**
 * A game's releases, newest first, each with the ways Kryoto can get it. Our
 * own copy leads (fastest, and checked against its hash); mirrors the app
 * resolves by itself follow, API hosts before page hosts.
 */
export async function fetchReleases(slug: string): Promise<KryoRelease[]> {
  const res = await fetch(await catalogApiUrl(`/api/games/${encodeURIComponent(slug)}/downloads`))
  if (!res.ok) throw new Error(res.status === 404 ? `kryo.to has no game at /game/${slug}.` : `kryo.to answered ${res.status}.`)
  const json = (await res.json()) as {
    releases?: RawRelease[]
    version?: string
    download_size?: string | null
    links?: RawLink[]
  }
  // A kryo.to from before `releases`: the current release is all it names.
  const raw: RawRelease[] = json.releases ?? [
    { id: 'current', version: json.version ?? '', download_size: json.download_size ?? null, primary: true, links: json.links ?? [] },
  ]
  const external = raw.flatMap((r) => (r.links ?? []).map((l) => l.url ?? '')).filter((u) => /^https?:\/\//.test(u))
  const known = new Map<string, { host: string; kind: string } | null>()
  if (external.length) {
    const kinds = await call<({ host: string; kind: string } | null)[]>('mirror_hosts', { urls: external }).catch(() => [])
    external.forEach((u, i) => known.set(u, kinds[i] ?? null))
  }
  return raw.map((r) => {
    const links = r.links ?? []
    const ours = links.some((l) => l.url?.startsWith('/api/download/'))
    const mirrors: ReleaseSource[] = []
    const elsewhere: { host: string; url: string }[] = []
    for (const l of links) {
      if (!l.url || l.url.startsWith('/')) continue
      const k = known.get(l.url)
      if (k) mirrors.push({ kind: 'mirror', host: k.host, url: l.url, page: k.kind === 'page' })
      else elsewhere.push({ host: l.host ?? 'Mirror', url: l.url })
    }
    mirrors.sort((a, b) => Number(a.kind === 'mirror' && a.page) - Number(b.kind === 'mirror' && b.page))
    return {
      id: String(r.id),
      version: r.version ?? '',
      label: r.label ?? null,
      source: r.source ?? null,
      downloadSize: r.download_size ?? null,
      createdAt: r.created_at ?? null,
      primary: !!r.primary,
      archived: !!r.archived,
      sources: [...(ours && !r.archived ? [{ kind: 'ours' } as const] : []), ...mirrors],
      elsewhere,
    }
  })
}

export type GameStateEvent = { id: string; running: boolean; seconds: number | null; code: number | null; stopped?: boolean }

export const BLANK_GAME: LibraryGame = {
  id: '',
  title: '',
  slug: null,
  cover: null,
  hero: null,
  installDir: '',
  executable: '',
  defaultArgs: '',
  entries: [],
  source: null,
  preferredEntry: null,
  askEachTime: false,
  launchOptions: '',
  compatTool: null,
  applyOverrides: true,
  playtimeSeconds: 0,
  lastPlayed: null,
  addedAt: 0,
  version: null,
  short: null,
  developer: null,
  nsfw: false,
}

/** One launch's log (src-tauri/src/game_logs.rs). */
export type GameLogInfo = { name: string; size: number; modified: number }

export const gameLogs = {
  list: (id: string) => call<GameLogInfo[]>('game_logs', { id }),
  read: (id: string, name: string) => call<string>('game_log_read', { id, name }),
  folder: (id: string) => call<string>('game_logs_folder', { id }),
  /** The game's Wine prefix on Linux; null on Windows or before its first start. */
  prefix: (id: string) => call<string | null>('library_prefix_folder', { id }),
}

export type UpdateBackup = {
  from_version: string | null
  to_version: string | null
  created_at: number
  files: number
  bytes: number
  dir: string
  state: 'applying' | 'applied' | 'rolled_back'
}

export const playtime = {
  /** Before Play: hours the folder or the account remembers beyond this PC's count. */
  offer: (id: string, accountSeconds: number | null) => call<PlaytimeOffer | null>('playtime_offer', { id, accountSeconds }),
  answer: (id: string, answer: 'yes' | 'later' | 'no', seconds: number) => call<LibraryGame>('playtime_answer', { id, answer, seconds }),
}

export const library = {
  list: () => call<LibraryGame[]>('library_list'),
  /** Games Kryoto installed in the library folders that the library has lost (their `.kryoto-game.json`). */
  findKryotoGames: () => call<FoundGame[]>('library_find_kryoto_games'),
  /** Put the picked ones back; how many were added. */
  restoreFound: (dirs: string[]) => call<number>('library_restore_found', { dirs }),
  add: async (exePath: string, game: Partial<LibraryGame>) => {
    const title = game.title?.trim() || exePath.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, '') || 'Game'
    const art = !game.slug && (!game.cover || !game.hero) ? await automaticArtwork(title) : null
    return call<LibraryGame>('library_add', { exePath, game: {
      ...BLANK_GAME, ...game, title,
      cover: game.cover ?? art?.cover ?? null,
      hero: game.hero ?? art?.hero ?? null,
      logo: game.logo ?? art?.logo ?? null,
      header: game.header ?? art?.header ?? null,
    } })
  },
  save: (game: LibraryGame) => call<LibraryGame>('library_save', { game }),
  /** Hide games or show them again, here and on the account. `slugs`: kryo.to games not on this PC. `adult`: every adult game on the account too. */
  setHidden: (ids: string[], hidden: boolean, more: { slugs?: string[]; adult?: boolean } = {}) =>
    call<void>('library_set_hidden', { ids, slugs: more.slugs ?? [], hidden, adult: more.adult ?? false }),
  /** The last update's backup, while its copies exist (src-tauri game_update.rs). */
  updateBackup: (slug: string) => call<UpdateBackup | null>('game_update_backup', { slug }),
  /** Put the build from before the last update back. */
  undoUpdate: (slug: string) => call<void>('game_update_undo', { slug }),
  refreshArtwork: (id: string, art: { cover: string | null; hero: string | null; logo?: string | null; header?: string | null }) =>
    call<LibraryGame>('library_artwork', { id, ...art, logo: art.logo ?? null, header: art.header ?? null }),
  /** `deletePrefix`: on Linux, its Wine prefix goes too. */
  remove: (id: string, deleteFiles: boolean, deletePrefix = false) => call<boolean>('library_remove', { id, deleteFiles, deletePrefix }),
  /** The game is somewhere else now: `exePath` is what starts it. Only its place changes. */
  relocate: (id: string, exePath: string) => call<LibraryGame>('library_relocate', { id, exePath }),
  /** Manage > Rename folder: the game's folder in plain letters, where it is (src-tauri library.rs). */
  plainFolder: (id: string) => call<LibraryGame>('library_plain_folder', { id }),
  folderExists: (path: string) => call<boolean>('folder_exists', { path }),
  /** `joinLobby`: a Steam lobby id from a game invite (Kryoto Online games). */
  launch: (id: string, entry: number | null, joinLobby?: string) => call<void>('game_launch', { id, entry, joinLobby: joinLobby ?? null }),
  /** "Play in VR": the same start with the headset's runtime made ready first (src-tauri vr.rs). */
  launchVr: (id: string, entry: number | null) => call<void>('game_launch_vr', { id, entry }),
  preview: (game: LibraryGame, entry: number | null) => call<string>('game_launch_preview', { game, entry }),
  running: () => call<string[]>('game_running'),
  stop: (id: string) => call<void>('game_stop', { id }),
  diskSize: (installDir: string) => call<number>('game_disk_size', { installDir }),
  /**
   * Show a folder. `create` for a library folder that may not exist yet; a
   * missing game folder shows the nearest one above it. Never rejects: a
   * folder that cannot be shown is logged, not thrown at the window.
   */
  openFolder: (path: string, create = false) =>
    call<void>('open_folder', { path, create }).catch((e) => logWarn('library', `open folder: ${errorText(e)}`)),
  /** The languages this build says it has; empty when it does not say. */
  languages: (installDir: string) => call<string[]>('game_languages', { installDir }),
  /** Where the shortcut went: `desktop`, or `menu` on a Linux desktop with no desktop folder. */
  shortcut: (id: string) => call<'desktop' | 'menu'>('game_shortcut', { id }),
  onState: (fn: (e: GameStateEvent) => void) => on<GameStateEvent>('game-state', fn),
  onChanged: (fn: () => void) => on<unknown>('library-changed', fn),
  addonUndo: (gameId: string, file: string) => call<LibraryGame>('addon_undo', { gameId, file }),
  /** `null` when the game can use Kryoto Online, else why it cannot. */
  onlineCheck: (gameId: string) => call<string | null>('online_check', { gameId }),
  onlineApply: (gameId: string) => call<LibraryGame>('online_apply', { gameId }),
  /** All clients share the same emulator repair/undo engine. */
  repairApply: (gameId: string, source: string) => call('repair_apply', { gameId, source }),
  onlineUndo: (gameId: string) => call<LibraryGame>('online_undo', { gameId }),
}

/* ── kryo.to ─────────────────────────────────────────────── */

export type CatalogGame = {
  slug: string
  title: string
  cover: string | null
  hero: string | null
  logo: string | null
  header: string | null
  screenshots: string[]
  executable: string
  defaultArgs: string
  entries: LaunchEntry[]
  source: string | null
  version: string | null
  short: string | null
  developer: string | null
  nsfw: boolean
  /** VR only, VR as well as flat, or neither (null). */
  vr: VrKind | null
}

/** A kryo.to game page link or a bare slug, as a slug. */
export function slugFrom(input: string): string | null {
  const text = input.trim()
  if (!text) return null
  const fromUrl = text.match(/kryo\.to\/game\/([a-z0-9-]+)/i)
  if (fromUrl?.[1]) return fromUrl[1].toLowerCase()
  return /^[a-z0-9-]+$/i.test(text) ? text.toLowerCase() : null
}

/**
 * A release's facts from kryo.to's public game API: Steam's launch entries,
 * the exe and arguments staff picked, art, version, and the source label
 * (which says whether Wine needs DLL overrides).
 */
export async function fetchCatalogGame(slug: string): Promise<CatalogGame> {
  const res = await fetch(await catalogApiUrl(`/api/games/${encodeURIComponent(slug)}`))
  if (res.status === 404) throw new Error(`kryo.to has no game at /game/${slug}.`)
  if (!res.ok) throw new Error(`kryo.to answered ${res.status}.`)
  const { game, art } = (await res.json()) as { game: Record<string, unknown>; art?: Record<string, unknown> }
  const artUrl = (k: string) => (typeof art?.[k] === 'string' && (art[k] as string).trim() ? (art[k] as string).trim() : null)
  const str = (v: unknown) => (typeof v === 'string' && v.trim() ? v.trim() : null)
  const available = (game.game_launch_options as { available?: LaunchEntry[] } | null)?.available
  // A Steam branch listed as its own game carries a suffix (GoreBox's
  // "2027330x"); Steam's art is under the number alone.
  const appid = str(game.steam_appid)?.match(/^\d+/)?.[0] ?? null
  return {
    slug,
    title: str(game.title) ?? slug,
    // `art` is what kryo.to resolved from Steam (real, hashed URLs); the
    // legacy appid paths are only a last guess, and they 404 for new games.
    cover: artUrl('capsule') ?? str(game.cover_vertical) ?? str(game.cover),
    hero:
      artUrl('hero') ??
      str(game.hero_image_override) ??
      (appid ? `https://cdn.cloudflare.steamstatic.com/steam/apps/${appid}/library_hero.jpg` : str(game.cover_horizontal)),
    logo: artUrl('logo'),
    header: artUrl('header') ?? str(game.cover_horizontal) ?? str(game.cover),
    screenshots: Array.isArray(art?.screenshots) ? (art.screenshots as unknown[]).filter((u): u is string => typeof u === 'string') : [],
    executable: str(game.game_executable_path) ?? '',
    defaultArgs: str(game.game_executable_args) ?? '',
    entries: Array.isArray(available) ? available.filter(isWindowsEntry) : [],
    source: str(game.source),
    version: str(game.version),
    short: str(game.short),
    developer: str(game.developer),
    nsfw: game.nsfw === true,
    vr: vrKind(game.vr),
  }
}

/** Steam's logo art for the game page, from the hero URL's app id. */
export function logoFor(game: LibraryGame): string | null {
  const m = (game.hero ?? game.cover ?? '').match(/\/apps\/(\d+)\//)
  return m ? `https://cdn.cloudflare.steamstatic.com/steam/apps/${m[1]}/logo.png` : null
}

/** Steam's landscape capsule, for the recent-games shelf. */
/**
 * The wide picture for a row or card, best first: the store header kryo.to
 * resolved, then the legacy Steam path (a guess that 404s for newer games),
 * then the cover.
 */
export function capsulesFor(game: { hero?: string | null; cover: string | null; header?: string | null; customCover?: string | null; customHero?: string | null }): (string | null)[] {
  const m = (game.hero ?? game.cover ?? '').match(/\/apps\/(\d+)\//)
  return [game.customCover ?? game.customHero ?? game.header ?? null, m ? `https://cdn.cloudflare.steamstatic.com/steam/apps/${m[1]}/header.jpg` : null, coverFor({ ...game, hero: game.hero ?? null })]
}

/* ── Launch entries ──────────────────────────────────────── */

const TYPE_LABELS: Record<string, string> = {
  vr: 'Play in VR',
  safemode: 'Safe mode',
  config: 'Configure',
  editor: 'Editor',
  server: 'Dedicated server',
  manual: 'Manual',
  option1: 'Alternative launch 1',
  option2: 'Alternative launch 2',
  option3: 'Alternative launch 3',
}

export function isWindowsEntry(e: LaunchEntry): boolean {
  const os = (e.oslist ?? '').toLowerCase()
  return !!e.executable && (!os || os.includes('windows'))
}

export function entryLabel(e: LaunchEntry): string {
  return e.description?.trim() || TYPE_LABELS[(e.type ?? '').toLowerCase()] || 'Play'
}

export function entryIsVr(e: LaunchEntry): boolean {
  return (e.type ?? '').toLowerCase() === 'vr' || /\bvr\b/i.test(entryLabel(e))
}

const norm = (path: string) => path.replace(/\\/g, '/').toLowerCase()

/**
 * What a plain Play starts: the game's own exe (`null`), the way it has always
 * started with nothing picked. A game with no exe on record starts with
 * Steam's first entry instead.
 */
export function normalEntry(game: LibraryGame): number | null {
  return game.executable.trim() || game.entries.length === 0 ? null : 0
}

/** One way to start a game. `entry` is what to pass to Play: `null` is the game's own exe. */
export type Way = { entry: number | null; label: string; detail: string; args: string; vr: boolean; normal: boolean }

/**
 * Every way to start the game, the plain one first and always there; the
 * launch entries (a VR mode, a config tool, a flag) are the ones under it.
 */
export function waysToPlay(game: LibraryGame): Way[] {
  const own = normalEntry(game)
  const same = (e: LaunchEntry) =>
    own === null && norm(e.executable) === norm(game.executable) && e.arguments.trim() === game.defaultArgs.trim()
  const ways: Way[] = []
  if (own === null) {
    // An entry that starts exactly what plain Play starts is the same way in;
    // it is not listed twice, but its name ("Play in VR") is kept.
    const twin = game.entries.find(same)
    ways.push({
      entry: null,
      label: twin ? entryLabel(twin) : 'Play',
      detail: [game.executable.trim(), game.defaultArgs.trim()].filter(Boolean).join(' '),
      args: game.defaultArgs.trim(),
      vr: twin ? entryIsVr(twin) : false,
      normal: true,
    })
  }
  game.entries.forEach((e, i) => {
    if (same(e)) return
    ways.push({
      entry: i,
      label: entryLabel(e),
      detail: [e.executable, e.arguments.trim()].filter(Boolean).join(' '),
      args: e.arguments.trim(),
      vr: entryIsVr(e),
      normal: i === own,
    })
  })
  return ways
}

/** Whether there is any way to start the game besides the plain one. */
export function hasChoice(game: LibraryGame): boolean {
  return waysToPlay(game).length > 1
}

/** The entry Play starts with no question: the one picked, else the plain one. */
export function startEntry(game: LibraryGame): number | null {
  if (game.preferredEntry != null && game.entries[game.preferredEntry]) return game.preferredEntry
  return normalEntry(game)
}

/** What Play starts, or `'ask'` when the player asked to be asked. */
/**
 * The game with `picked` (an .exe chosen by hand) as what Play starts: relative
 * to its folder when it is inside it, otherwise in a folder of its own. A file
 * chosen by hand wins over a Steam entry and over "Ask every time".
 */
export function withExecutable(game: LibraryGame, picked: string): LibraryGame {
  const norm = picked.replace(/\\/g, '/')
  const root = game.installDir.replace(/\\/g, '/').replace(/\/+$/, '')
  const plain = { ...game, preferredEntry: null, askEachTime: false }
  if (norm.toLowerCase().startsWith(`${root.toLowerCase()}/`)) return { ...plain, executable: norm.slice(root.length + 1) }
  const cut = norm.lastIndexOf('/')
  return { ...plain, installDir: picked.slice(0, cut), executable: norm.slice(cut + 1) }
}

export function playTarget(game: LibraryGame): number | null | 'ask' {
  if (game.askEachTime && hasChoice(game)) return 'ask'
  return startEntry(game)
}

/* ── Launch option presets (the Steam non-Steam-game guide) ── */

export const PRESETS = [
  {
    id: 'kryoto-online',
    label: 'Kryoto Online',
    line: 'WINEDLLOVERRIDES="steam_api64=n,b;kryotoO=n,b;photon_universal=n,b" %command%',
  },
  {
    id: 'online-fix',
    label: 'Online-Fix',
    line: 'WINEDLLOVERRIDES="OnlineFix64=n;SteamOverlay64=n;winmm=n,b;dnet=n;steam_api64=n" %command%',
  },
] as const

/** Which preset a release's source label calls for, if any. */
export function presetFor(source: string | null): (typeof PRESETS)[number] | null {
  const s = (source ?? '').toLowerCase()
  if (s.includes('kryoto online')) return PRESETS[0]
  if (/online-?fix|\bofme\b/.test(s)) return PRESETS[1]
  return null
}

/** The line for Steam's LAUNCH OPTIONS box, when played through Steam. */
export function steamLine(game: LibraryGame, entry: number | null): string {
  const args = entry != null ? (game.entries[entry]?.arguments ?? '') : game.defaultArgs
  const preset = presetFor(game.source)
  return [preset ? preset.line : '%command%', args.trim()].filter(Boolean).join(' ')
}

export const isWindowsHost = () => /Windows/i.test(navigator.userAgent)

/** The last part of a path (a game's folder name). */
export const folderName = (path: string) => path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() ?? path
/**
 * Plain letters only. Some games will not start from a folder with `’` or `ö`
 * in its name; new installs never get one (downloads.rs `plain_name`), and
 * Manage > Rename folder fixes an older one. The game's own folder can sit a
 * level above its root (`Game’s/bin`), so both are looked at.
 */
export const plainPath = (path: string) => !/[^\x20-\x7e]/.test(path.replace(/[\\/]+$/, '').split(/[\\/]/).slice(-2).join('/'))

/** What Play can start, for the file picker: a game's own start script too. */
export const startFileTypes = () => (isWindowsHost() ? ['exe', 'bat', 'cmd'] : ['exe', 'bat', 'cmd', 'sh'])
